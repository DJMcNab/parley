// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Text shaping implementation using `harfrust`for shaping
//! and `icu` for text analysis.

use alloc::vec::Vec;
use parley_engine::shape::{CharCluster, Coverage, FontCoverageCache};
use parley_engine::{Analysis, FontInstance, FontInstanceRef, ShapeOptions, Shaper};
use smallvec::SmallVec;

use super::layout::Layout;
use super::resolve::{ResolveContext, ResolvedStyle};
use super::style::{Brush, FontFeature};
use crate::inline_box::LayoutInlineBox;
use crate::util::{nearly_eq, nearly_zero};
use crate::{FontRequest, FontSource, Presentation, Spacing};

use parlance::{BidiLevel, Tag};

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

pub(crate) fn shape_text<'a, B: Brush, F: FontSource>(
    rcx: &'a ResolveContext,
    fcx: &'a mut F,
    styles: &'a [ResolvedStyle<B>],
    inline_boxes: &[LayoutInlineBox],
    analysis: &Analysis,
    char_style_indices: &[u16],
    scx: &mut Shaper,
    font_coverage: &mut FontCoverageCache,
    text: &str,
    layout: &mut Layout<B>,
    style_features: &'a mut Vec<SmallVec<[FontFeature; 8]>>,
) {
    // Do nothing if there is no text or styles (there should always be a default style)
    if text.is_empty() || styles.is_empty() {
        // Empty text may still have out-of-flow boxes.
        for box_idx in 0..inline_boxes.len() {
            layout.data.push_out_of_flow_box(box_idx, BidiLevel::new(0));
        }
        return;
    }

    let mut out_of_flow_box_iter = inline_boxes
        .iter()
        .filter(|b| !b.is_inline_object())
        .map(|b| &b.inline_box)
        .peekable();

    // Merge font features with letter-spacing ligature suppression.
    //
    // TODO: This copy is slightly unfortunate (though solvable). It's required currently,
    // because the iterator providing `ShapeOptions` has to provide a borrowed `&'a [FontFeature]`,
    // which cannot be tied to the lifetime of the call to `Iterator::next`. What we'd need is a
    // lending iterator (i.e., we probably just need to let `parley_engine` take some trait
    // providing the items).
    style_features.clear();
    style_features.extend(styles.iter().map(|style| {
        let style_features = rcx.features(style.font_features).unwrap_or(&[]);
        if !nearly_zero(style.letter_spacing) {
            // Later values override earlier values.
            OPTIONAL_LIGATURES_OFF
                .into_iter()
                .chain(style_features.iter().copied())
                .collect()
        } else {
            style_features.iter().copied().collect()
        }
    }));
    let style_features = &*style_features;

    // Split when shaping-relevant style properties change and at out-of-flow boxes.
    let items = {
        // TODO: we currently walk characters here, but we could instead just walk boundaries of
        // styles and inline boxes.
        let char_count = char_style_indices.len();

        // The first character of the item currently being built.
        let mut item_start_char = 0_usize;

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

                // Split at inlines boxes, so each box falls on a shaping boundary.
                //
                // We loop because there may be multiple boxes at this index.
                let mut split = false;
                while let Some(inline_box) = out_of_flow_box_iter.peek() {
                    if inline_box.index < byte_index {
                        // Inline boxes *before* this index are popped (this occurs if the itemizer
                        // split a run and we were not called, such as at a bidi boundary).
                        out_of_flow_box_iter.next();
                    } else if inline_box.index == byte_index {
                        out_of_flow_box_iter.next();
                        split = true;
                    } else {
                        break;
                    }
                }

                if split {
                    break char_index;
                }

                let style_index = char_style_indices[char_index];
                if style_index != item_style_index {
                    let style = &styles[usize::from(style_index)];
                    split = !nearly_eq(style.font_size, item_style.font_size)
                        || style.locale != item_style.locale
                        || style.font_variations != item_style.font_variations
                        || style.font_features != item_style.font_features
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
                    features: &style_features[item_style_index as usize],
                    variations: rcx.variations(item_style.font_variations).unwrap_or(&[]),
                },
            })
        })
    };

    let font_selector = FontSelector {
        source: fcx,
        styles,
        coverage: font_coverage,
        selected: None,
    };

    scx.shape_text(
        text,
        analysis,
        char_style_indices,
        items,
        font_selector,
        false,
        &mut layout.data.shaped_text,
    );

    // Out-of-flow boxes are laid out as items of their own.
    let mut out_of_flow_box_iter = inline_boxes
        .iter()
        .enumerate()
        .filter(|(_, b)| !b.is_inline_object())
        .map(|(box_idx, b)| (box_idx, &b.inline_box))
        .peekable();
    for shaped_run_idx in 0..layout.data.shaped_text.runs().len() {
        let shaped_run = &layout.data.shaped_text.runs()[shaped_run_idx];
        let run_text_byte_start = shaped_run.range.byte_range.start;
        let run_style_index = char_style_indices[shaped_run.range.char_range.start];
        let run_style = &styles[usize::from(run_style_index)];

        // Push out-of-flow boxes positioned before the start of this item.
        //
        // TODO: this gives the box the bidi level of the previous run. At direction boundaries it
        // puts the box on the wrong side, e.g. at the start of an RTL paragraph, or at the end of
        // an LTR one that ends in RTL text.
        let prev_bidi_level = if shaped_run_idx > 0 {
            layout.data.shaped_text.runs()[&shaped_run_idx - 1].bidi_level
        } else {
            BidiLevel::new(0)
        };
        while let Some((box_idx, inline_box)) = out_of_flow_box_iter.peek() {
            if inline_box.index <= run_text_byte_start {
                layout.data.push_out_of_flow_box(*box_idx, prev_bidi_level);
                out_of_flow_box_iter.next();
            } else {
                break;
            }
        }

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

    // Process any remaining inline boxes whose index is greater than the length of the text
    //
    // Give the box the same bidi level as the last text run (or else default to 0 if there is no
    // text run).
    let bidi_level = layout
        .data
        .shaped_text
        .runs()
        .last()
        .map(|r| r.bidi_level)
        .unwrap_or(BidiLevel::new(0));
    for (box_idx, _inline_box) in out_of_flow_box_iter {
        layout.data.push_out_of_flow_box(box_idx, bidi_level);
    }
}

struct FontSelector<'a, B: Brush, F: FontSource> {
    source: &'a mut F,
    styles: &'a [ResolvedStyle<B>],
    coverage: &'a mut FontCoverageCache,
    selected: Option<FontInstance>,
}

impl<B: Brush, F: FontSource> parley_engine::FontSelector for FontSelector<'_, B, F> {
    fn select_font(
        &mut self,
        segment: &parley_engine::itemize::Segment,
        options: &ShapeOptions<'_>,
        cluster: &mut CharCluster,
    ) -> Option<FontInstanceRef<'_>> {
        let request = FontRequest {
            id: self.styles[usize::from(cluster.style_index())].font_selection,
            script: segment.script,
            language: options.language.as_ref(),
            presentation: if cluster.is_emoji() {
                Presentation::Emoji
            } else {
                Presentation::Text
            },
        };
        let mut best = self.selected.take();
        let mut found = false;
        let mut best_coverage = Coverage::NONE;
        let coverage_cache = &mut self.coverage;
        self.source.candidates(&request, |font| {
            let Some(coverage) = coverage_cache.coverage(font, cluster) else {
                return core::ops::ControlFlow::Continue(());
            };
            if !found || coverage > best_coverage {
                if let Some(best) = &mut best {
                    if best.as_ref() != font {
                        best.font.data = font.data.clone();
                        best.font.index = font.index;
                        best.variations.clear();
                        best.variations.extend_from_slice(font.variations);
                        best.synthetic_style = font.synthetic_style;
                    }
                } else {
                    best = Some(FontInstance::from(font));
                }
                found = true;
                best_coverage = coverage;
            }
            if coverage.is_complete() {
                core::ops::ControlFlow::Break(())
            } else {
                core::ops::ControlFlow::Continue(())
            }
        });
        self.selected = if found {
            best
        } else {
            self.source.last_resort(&request).map(Into::into)
        };
        self.selected.as_ref().map(FontInstance::as_ref)
    }
}
