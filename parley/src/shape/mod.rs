// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text shaping implementation using `harfrust`for shaping
//! and `icu` for text analysis.

use core::range::Range;

use alloc::vec::Vec;
use parley_engine::shape::{CharCluster, Coverage};
use parley_engine::{Analysis, FontInstance, FontInstanceRef, RangedFeature, ShapeOptions, Shaper};

use super::layout::Layout;
use super::resolve::{ResolveContext, ResolvedStyle};
use super::style::{Brush, FontFeature};
use crate::FloatedBox;
use crate::util::{nearly_eq, nearly_zero};
use crate::{FontContext, FontData, Spacing};

use fontique::{self, Query, QueryFamily, QueryFont};
use parlance::{GenericFamily, Tag};

/// If these font features are passed to the shaper, optional ligatures are not applied.
///
/// Required ligation such as is common in cursive scripts' fonts is still applied.
///
/// Following [CSS Text 3 § 7.2][css-letter-spacing], pass these features if you are going to apply
/// something like `letter-spacing`. Unfortunately, that specification doesn't say which font
/// features specifically should be disabled. Here, we disable the OpenType features for "common",
/// "discretionary" and "historic" ligatures as given by [CSS Fonts 4 § 6.4][css-fonts-ligatures].
///
/// As of writing, this list matches Gecko. Blink additionally disables contextual alternates
/// (`calt`), but that's perhaps not desirable: disabling contextual alternates potentially makes
/// text render wrong.
///
/// [css-letter-spacing]: https://www.w3.org/TR/css-text-3/#letter-spacing-property
/// [css-fonts-ligatures]: https://www.w3.org/TR/css-fonts-4/#font-variant-ligatures-prop
const OPTIONAL_LIGATURES_OFF: [FontFeature; 4] = [
    FontFeature::new(Tag::new(b"liga"), 0),
    FontFeature::new(Tag::new(b"clig"), 0),
    FontFeature::new(Tag::new(b"dlig"), 0),
    FontFeature::new(Tag::new(b"hlig"), 0),
];

pub(crate) fn shape_text<'a, B: Brush>(
    rcx: &'a ResolveContext,
    fcx: &'a mut FontContext,
    styles: &'a [ResolvedStyle<B>],
    floated_boxes: &[FloatedBox],
    shaping_splits: &[usize],
    analysis: &Analysis,
    char_style_indices: &[u16],
    scx: &mut Shaper,
    text: &str,
    layout: &mut Layout<B>,
    ranged_features: &mut Vec<RangedFeature>,
) {
    let scale = layout.data.scale;

    // Do nothing if there is no text or styles (there should always be a default style)
    if text.is_empty() || styles.is_empty() {
        return;
    }

    let mut fq = fcx.collection.query(&mut fcx.source_cache);

    let mut floated_box_iter = floated_boxes.iter().peekable();

    // Collect the font features of each span of characters, including the ligature suppression of
    // letter spacing and the punctuation trimmed by `text-spacing-trim`.
    //
    // Adjacent styles often only differ in properties that are irrelevant here (like the brush).
    // Spans of such styles are merged, so that features are usually constant over a shaped run.
    ranged_features.clear();
    let trim_sites = &mut layout.data.trim_sites;
    trim_sites.collect(
        text,
        rcx,
        styles,
        char_style_indices,
        analysis.bidi_levels(),
    );
    let mut trims = trim_sites.trims().peekable();
    let mut span_start = 0_usize;
    for span in char_style_indices.chunk_by(|&a, &b| {
        a == b || {
            let (a, b) = (&styles[usize::from(a)], &styles[usize::from(b)]);
            a.font_features == b.font_features
                && nearly_zero(a.letter_spacing) == nearly_zero(b.letter_spacing)
        }
    }) {
        let span_end = span_start + span.len();
        let char_range = Range::from(span_start as u32..span_end as u32);
        span_start = span_end;

        let style = &styles[usize::from(span[0])];
        let ligatures_off: &[FontFeature] = if nearly_zero(style.letter_spacing) {
            &[]
        } else {
            &OPTIONAL_LIGATURES_OFF
        };
        let style_features = rcx.features(style.font_features).unwrap_or(&[]);
        // Later features override earlier features.
        ranged_features.extend(ligatures_off.iter().chain(style_features).map(|&feature| {
            RangedFeature {
                feature,
                char_range,
            }
        }));
        // Trimming takes precedence over the style's own features, so it comes after them. It is
        // added per span to keep the features sorted by the start of their ranges.
        ranged_features.extend(core::iter::from_fn(|| {
            trims.next_if(|trim| trim.char_range.start < char_range.end)
        }));
    }
    // This borrows the layout.
    drop(trims);

    // Split when shaping-relevant style properties change and at floated boxes. Font features are
    // not among these: they can change within an item.
    let items = {
        // TODO: we currently walk characters here, but we could instead just walk boundaries of
        // styles, explicit splits, and floated boxes.
        let char_count = char_style_indices.len();

        // The first character of the item currently being built.
        let mut item_start_char = 0_usize;

        let mut shaping_splits = shaping_splits.iter().copied().peekable();

        // Positioned at `item_start_char`, i.e., the first character not yet covered by an item.
        let mut chars = text.char_indices().enumerate().peekable();
        core::iter::from_fn(move || {
            if item_start_char == char_count {
                return None;
            }

            let item_style_index = char_style_indices[item_start_char];
            let item_style = &styles[usize::from(item_style_index)];

            // Items are at least one character long, therefore the item's own first character is
            // never a split point.
            chars.next();

            let char_end = loop {
                let Some(&(char_index, (byte_index, _))) = chars.peek() else {
                    // End of text.
                    break char_count;
                };

                // Split at floated boxes, so each box falls on a shaping boundary.
                //
                // We loop because there may be multiple boxes at this index.
                let mut split = false;
                while let Some(floated_box) = floated_box_iter.peek() {
                    if floated_box.index < byte_index {
                        // Floated boxes *before* this index are popped (this occurs if the itemizer
                        // split a run and we were not called, such as at a bidi boundary).
                        floated_box_iter.next();
                    } else if floated_box.index == byte_index {
                        floated_box_iter.next();
                        split = true;
                    } else {
                        break;
                    }
                }

                split |= shaping_splits.next_if_eq(&byte_index).is_some();

                if split {
                    break char_index;
                }

                let style_index = char_style_indices[char_index];
                if style_index != item_style_index {
                    let style = &styles[usize::from(style_index)];
                    split = !nearly_eq(style.font_size, item_style.font_size)
                        || style.locale != item_style.locale
                        || style.font_variations != item_style.font_variations
                        || !nearly_eq(style.letter_spacing, item_style.letter_spacing)
                        || !nearly_eq(style.word_spacing, item_style.word_spacing);
                }

                if split {
                    break char_index;
                }

                chars.next();
            };

            item_start_char = char_end;
            Some(parley_engine::itemize::Item {
                char_end: char_end as u32,
                options: ShapeOptions {
                    language: item_style.locale,
                    font_size: item_style.font_size,
                    // TODO: Keep the logical font size in `ResolvedStyle` instead of
                    // dividing the scale back out.
                    point_size: item_style.font_size / scale,
                    variations: rcx.variations(item_style.font_variations).unwrap_or(&[]),
                },
            })
        })
    };

    let font_selector = FontSelector::new(&mut fq, rcx, styles);

    scx.shape_text(
        text,
        analysis,
        char_style_indices,
        items,
        ranged_features,
        font_selector,
        false,
        &mut layout.data.shaped_text,
    );

    for shaped_run_idx in 0..layout.data.shaped_text.runs().len() {
        let shaped_run = &layout.data.shaped_text.runs()[shaped_run_idx];
        let run_style_index = char_style_indices[shaped_run.range.char_range.start];
        let run_style = &styles[usize::from(run_style_index)];

        layout.data.process_shaped_run(
            shaped_run_idx,
            // TODO: should we get the *item's* style here?
            //
            // Using the run's style for word and letter spacing is probably correct (and in fact,
            // we probably shouldn't itemize on them; we should just ensure we don't ligate if
            // they're non-zero).
            run_style,
            Spacing::new(run_style.word_spacing, run_style.letter_spacing),
            char_style_indices,
        );
    }
}

/// The font to use if a font query doesn't return any font candidates at all.
#[derive(Debug)]
enum LastResortFont {
    Unresolved,
    Resolved(FontInstance),
    Unavailable,
}

struct FontSelector<'a, 'b, B: Brush> {
    query: &'b mut Query<'a>,
    fonts_id: Option<usize>,
    rcx: &'a ResolveContext,
    styles: &'a [ResolvedStyle<B>],
    style_index: u16,
    attrs: fontique::Attributes,

    /// Whether [`Self::query`] yields any font, cached.
    query_yields_font: Option<bool>,

    /// The font to use if [`Self::query`] doesn't return any font.
    last_resort_font: LastResortFont,
}

impl<'a, 'b, B: Brush> FontSelector<'a, 'b, B> {
    /// Construct a new `FontSelector`.
    ///
    /// If `query` ends up not returning a font for a query, the `last_resort_font` is returned
    /// instead.
    fn new(
        query: &'b mut Query<'a>,
        rcx: &'a ResolveContext,
        styles: &'a [ResolvedStyle<B>],
    ) -> Self {
        let attrs = fontique::Attributes::default();

        Self {
            query,
            fonts_id: None,
            rcx,
            styles,
            style_index: 0,
            attrs,
            query_yields_font: None,
            last_resort_font: LastResortFont::Unresolved,
        }
    }
}

impl<'a, 'b, B: Brush> parley_engine::FontSelector for FontSelector<'a, 'b, B> {
    fn begin_segment(
        &mut self,
        segment: &parley_engine::itemize::Segment,
        options: &ShapeOptions<'_>,
    ) {
        self.query.set_fallbacks(fontique::FallbackKey::new(
            segment.script,
            options.language.as_ref(),
        ));
        self.query_yields_font = None;
    }

    fn select_font(
        &mut self,
        _segment: &parley_engine::itemize::Segment,
        _options: &ShapeOptions<'_>,
        cluster: &mut CharCluster,
    ) -> Option<FontInstanceRef<'_>> {
        let style_index = cluster.style_index();
        let is_emoji = cluster.is_emoji();

        if style_index != self.style_index || is_emoji || self.fonts_id.is_none() {
            self.style_index = style_index;
            let style = &self.styles[style_index as usize];

            let fonts_id = style.font_family.id();
            let fonts = self.rcx.stack(style.font_family).unwrap_or(&[]);
            let fonts = fonts.iter().copied().map(QueryFamily::Id);
            if is_emoji {
                use core::iter::once;
                let emoji_family = QueryFamily::Generic(GenericFamily::Emoji);
                self.query.set_families(fonts.chain(once(emoji_family)));
                self.fonts_id = None;
                self.query_yields_font = None;
            } else if self.fonts_id != Some(fonts_id) {
                self.query.set_families(fonts);
                self.fonts_id = Some(fonts_id);
                self.query_yields_font = None;
            }

            let attrs = fontique::Attributes {
                width: style.font_width,
                weight: style.font_weight,
                style: style.font_style,
            };
            if self.attrs != attrs {
                self.query.set_attributes(attrs);
                self.attrs = attrs;
                self.query_yields_font = None;
            }
        }

        // Fall back to the last-resort font if the query doesn't yield a font (with a charmap) at
        // all. This is checked before the actual query below due to lifetime issues. While this is
        // not *too* expensive, once the new borrow checker (Polonius) is stable, we could remove
        // this check and do the last-resort scan after the failed query:
        // <https://blog.rust-lang.org/2026/08/04/enabling-polonius-alpha-on-nightly/>.
        let query = &mut *self.query;
        let query_yields_font = *self.query_yields_font.get_or_insert_with(|| {
            let mut yields_font = false;
            query.matches_with(|font| {
                if font.charmap().is_some() {
                    yields_font = true;
                    fontique::QueryStatus::Stop
                } else {
                    fontique::QueryStatus::Continue
                }
            });
            yields_font
        });
        if !query_yields_font {
            if matches!(self.last_resort_font, LastResortFont::Unresolved) {
                if let Some(font) = any_font(self.query) {
                    self.last_resort_font = LastResortFont::Resolved(FontInstance {
                        font: FontData {
                            data: font.blob,
                            index: font.index,
                        },
                        synthesis: font.synthesis,
                    });
                } else {
                    self.last_resort_font = LastResortFont::Unavailable;
                }

                self.fonts_id = None;
            }

            return if let LastResortFont::Resolved(ref font) = self.last_resort_font {
                Some(font.as_ref())
            } else {
                None
            };
        }

        let mut selected_font = None;
        let mut best_coverage = Coverage::NONE;

        self.query.matches_with(|font| {
            // Only create the charmap if the character is the unicode range, as the the range check
            // is a lot cheaper than creating the charmap. That matters if families are split into
            // many subsets.
            let mut charmap = None;
            let coverage = cluster.calculate_coverage(|ch| {
                font.in_unicode_range(ch)
                    && charmap
                        .get_or_insert_with(|| font.charmap())
                        .as_ref()
                        .and_then(|charmap| charmap.map(ch))
                        // Any non-zero value indicates the existence of a glyph.
                        .is_some_and(|g| g != 0)
            });
            if let Some(None) = charmap {
                return fontique::QueryStatus::Continue;
            }
            if coverage > best_coverage {
                selected_font = Some(font);
                best_coverage = coverage;

                if coverage.is_complete() {
                    fontique::QueryStatus::Stop
                } else {
                    fontique::QueryStatus::Continue
                }
            } else {
                if selected_font.is_none() {
                    selected_font = Some(font);
                }
                fontique::QueryStatus::Continue
            }
        });

        selected_font.map(|font| FontInstanceRef {
            data: &font.blob,
            index: font.index,
            synthesis: &font.synthesis,
        })
    }
}

/// Just query for any generic family's font from the font collection.
///
/// This sets generic families on `query`, so callers need to make sure to set families back again.
/// This returns `None` if it doesn't find any fonts, but note this only checks generic families.
///
/// This can be used to have a font at hand for shaping, in case more sophisticated font querying
/// returns no fonts.
fn any_font(query: &mut Query<'_>) -> Option<QueryFont> {
    let mut found = None;

    // Try the system's generic text families.
    //
    // At this point, the font itself doesn't really matter: we mostly just need any font to work
    // with. This has a cost, as it extends our query's candidate font families with all the generic
    // families registered on the system. A better future may be to allow users to register some
    // last resort in `fontique`, or pass it directly to `parley`.
    query.set_families([
        QueryFamily::Generic(GenericFamily::SansSerif),
        QueryFamily::Generic(GenericFamily::Serif),
        QueryFamily::Generic(GenericFamily::Monospace),
    ]);
    query.matches_with(
        #[inline]
        |font: &QueryFont| {
            found = Some(font.clone());
            fontique::QueryStatus::Stop
        },
    );

    found
}
