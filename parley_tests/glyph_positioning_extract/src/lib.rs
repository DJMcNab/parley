// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Parley side of the glyph-positioning Chrome-parity comparison.
//!
//! This is the *only* place that side is constructed: both the (future) CI test and
//! the (future) fuzz loop call into it, so a drift between them can't silently
//! invalidate every golden. See `doc/glyph-positioning-chrome-parity-phase1.md`.

use std::sync::Arc;

use fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    BreakReason, CHROMIUM_LINE_BREAK_OVERRIDE, FontContext, FontFamily, Layout, LayoutContext,
    Line, LineHeight, PositionedLayoutItem, StyleProperty,
};
use parley_glyph_positioning_cases::{
    Case, FONTS, GlyphOutput, GlyphOutputBuilder, MAX_ADVANCE_EPSILON, Style, ceil_to_layout_unit,
    postscript_name_from_bytes, quantize_font_size,
};

/// Builds a [`FontContext`] with exactly [`FONTS`] registered and no system-font
/// fallback.
///
/// This is the *only* place `FontContext` is set up for this comparison, so callers
/// (the CI test and the fuzz loop) can't accidentally register fonts differently and
/// silently drift from each other — e.g. one enabling system-font fallback, which
/// could substitute a font Chrome never used.
#[must_use]
pub fn font_context() -> FontContext {
    let mut collection = Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    });
    for font in FONTS {
        collection.register_fonts(Blob::new(Arc::new(font.bytes.to_vec())), None);
    }
    FontContext {
        collection,
        source_cache: SourceCache::default(),
    }
}

/// Lays out `case` with Parley, applying the same Chromium quantizations the harness
/// models (see "Chromium behaviour modelled" in the Phase 1 doc): each run's font size
/// is truncated the way Blink's font cache truncates it, each run's line height is set
/// as the same absolute length the harness's CSS applies (`StyleProperty::LineHeight(
/// LineHeight::Absolute(_))`, matching `harness_css::run_css`'s `line-height:_px`), and
/// lines are broken against `case.width` plus [`MAX_ADVANCE_EPSILON`].
#[must_use]
pub fn layout(
    case: &Case,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) -> Layout<()> {
    let text: String = case.runs.iter().map(|run| run.text.as_str()).collect();

    let mut builder = layout_cx.ranged_builder(font_cx, &text, 1.0, true);
    builder.push_default(FontFamily::named(FONTS[0].family));

    let mut offset = 0;
    for run in &case.runs {
        let range = offset..offset + run.text.len();
        builder.push(
            StyleProperty::FontSize(quantize_font_size(run.font_size)),
            range.clone(),
        );
        builder.push(
            StyleProperty::LetterSpacing(run.letter_spacing),
            range.clone(),
        );
        builder.push(StyleProperty::WordSpacing(run.word_spacing), range.clone());
        builder.push(
            StyleProperty::LineHeight(LineHeight::Absolute(run.line_height)),
            range.clone(),
        );
        offset = range.end;
    }
    builder.set_line_break_override(Some(CHROMIUM_LINE_BREAK_OVERRIDE));
    let mut layout = builder.build(&text);
    layout.break_all_lines(Some(case.width + MAX_ADVANCE_EPSILON));
    layout
}

/// Extracts [`GlyphOutput`] from an already-broken `layout` of `case`.
///
/// The output is fragment-structured, because Blink's is: a line is a sequence of
/// fragments, each placed at the previous one's width rounded **up** onto
/// `LayoutUnit`'s 1/64 px grid (see [`ceil_to_layout_unit`]). Reproducing that here,
/// rather than tolerating it in the comparison, is what lets a case hold more than one
/// span per line and still be compared exactly. [`fragments_of`] documents what counts
/// as a fragment; that definition is the delicate part, not this recurrence.
///
/// Glyph offsets accumulate in **f64**, not via [`GlyphRun::positioned_glyphs`] (whose
/// f32 running offset would itself contribute more error than the 16.16-accumulation
/// drift this harness measures against Chrome) — see "Parley must accumulate in f64"
/// in the Phase 1 doc. The per-glyph advances themselves stay f32; only the
/// accumulator changes.
///
/// [`GlyphRun::positioned_glyphs`]: parley::GlyphRun::positioned_glyphs
#[must_use]
pub fn parley_output(layout: &Layout<()>, case: &Case) -> GlyphOutput {
    let bounds = span_bounds(case);
    let mut builder = GlyphOutputBuilder::default();

    for line in lines_with_glyphs(layout) {
        // Blink positions a line's fragments left to right from the line's own content
        // edge; taking that edge from the first fragment keeps whatever alignment
        // offset Parley applied, rather than assuming zero.
        let mut origin = f64::from(line_start(&line));
        let baseline = f64::from(line.metrics().baseline);

        for fragment in fragments_of(&line, layout, &bounds) {
            let font = fragment.run.font();
            let postscript_name =
                postscript_name_from_bytes(font.font.data.data(), font.font.index)
                    .expect("Parley-selected font must have a readable PostScript name");
            let style = builder.style_index(Style {
                postscript_name,
                font_size: fragment.run.font_size(),
            });

            builder.begin_fragment(origin, baseline, style);
            for glyph in &fragment.glyphs {
                builder.push_glyph(glyph.id, glyph.x, glyph.y);
            }
            origin += ceil_to_layout_unit(fragment.width);
        }
    }

    builder.build()
}

/// One fragment's worth of Parley output, before it is given an origin.
struct ParleyFragment<'a> {
    /// The Parley run the fragment's style is read from. Every glyph in a fragment
    /// shares a style, since a fragment is one span's glyphs.
    run: parley::Run<'a, ()>,
    /// Glyph offsets from the (not yet known) fragment origin.
    glyphs: Vec<parley::Glyph>,
    /// The fragment's own unrounded advance width — what Blink rounds up to place the
    /// next fragment.
    width: f64,
}

/// Splits one line into the fragments Blink would paint it as, in visual order.
///
/// Three rules decide where a fragment ends, and none of them is "wherever Parley
/// starts a new glyph run":
///
/// 1. **A fragment is one span, not one shaping run.** Blink's `InlineItemsBuilder`
///    splits at DOM boundaries; the `RunSegmenter`'s script and font segmentation
///    happens *inside* one item's `ShapeResult`. Parley's [`Line::items`] is the finer
///    split — one `GlyphRun` per script run — so consecutive glyph runs from the same
///    span are merged back together here. The sampling alphabet is deliberately
///    multi-script, so a single-span case routinely arrives as a dozen or more glyph
///    runs; snapping at each of them is wrong by far more than the snapping being
///    modelled.
/// 2. **A soft-wrapped line's hanging trailing whitespace is a fragment of its own** —
///    but only when the break splits a span. If the break lands exactly on a span
///    boundary there is nothing to split, and the whitespace stays inside that span's
///    fragment. The last glyph-bearing line's trailing whitespace is never split
///    either, even when Parley soft-broke after it into an empty trailing line.
/// 3. **Two adjacent spans with identical styles are two fragments to Blink, and
///    Parley cannot see the boundary** — [`Line::items`] splits on resolved style
///    index, so it hands both spans over as one glyph run. This is not modelled and
///    cannot be from here; case generation avoids producing it, and `compare` reports
///    it as a fragmentation mismatch if it ever arrives anyway.
fn fragments_of<'a>(
    line: &Line<'a, ()>,
    layout: &Layout<()>,
    bounds: &[usize],
) -> Vec<ParleyFragment<'a>> {
    let hanging = hanging_whitespace(line, layout, bounds);

    let mut fragments: Vec<ParleyFragment<'a>> = Vec::new();
    let mut current_span = None;
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            continue;
        };
        let span = span_of(glyph_run.run().text_range().start, bounds);
        if current_span != Some(span) {
            current_span = Some(span);
            fragments.push(ParleyFragment {
                run: *glyph_run.run(),
                glyphs: Vec::new(),
                width: 0.0,
            });
        }
        let fragment = fragments.last_mut().expect("just pushed");
        for glyph in glyph_run.glyphs() {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "offsets within one fragment are small; f64 is for accumulation only"
            )]
            fragment.glyphs.push(parley::Glyph {
                x: (fragment.width + f64::from(glyph.x)) as f32,
                ..glyph
            });
            fragment.width += f64::from(glyph.advance);
        }
    }

    let Some(hanging_width) = hanging else {
        return fragments;
    };
    let Some(last) = fragments.pop() else {
        return fragments;
    };
    let Some((head, tail)) = split_off_hanging(last, hanging_width) else {
        // The hanging run could not be identified from the advances alone. Leaving the
        // fragment whole reports as a position mismatch, which is the honest outcome —
        // silently guessing a split point would report as parity where there is none.
        return fragments;
    };
    fragments.push(head);
    fragments.push(tail);
    fragments
}

/// Splits `fragment` into the part Blink measures and the trailing whitespace it hangs,
/// or `None` if `hanging_width` doesn't line up with a whole number of trailing glyphs.
fn split_off_hanging(
    fragment: ParleyFragment<'_>,
    hanging_width: f64,
) -> Option<(ParleyFragment<'_>, ParleyFragment<'_>)> {
    // Advances are recovered from consecutive glyph offsets, then peeled off the end
    // until they account for the line's trailing whitespace. The epsilon absorbs only
    // the f32→f64 widening of a sum Parley itself computed in f32.
    const EPSILON: f64 = 1e-3;

    let mut peeled = 0;
    let mut peeled_width = 0.0;
    while peeled < fragment.glyphs.len() && peeled_width + EPSILON < hanging_width {
        let glyph = fragment.glyphs[fragment.glyphs.len() - 1 - peeled];
        let next_offset = fragment
            .glyphs
            .get(fragment.glyphs.len() - peeled)
            .map_or(fragment.width, |next| f64::from(next.x));
        peeled_width += next_offset - f64::from(glyph.x);
        peeled += 1;
    }
    if (peeled_width - hanging_width).abs() > EPSILON
        || peeled == 0
        || peeled >= fragment.glyphs.len()
    {
        return None;
    }

    let head_len = fragment.glyphs.len() - peeled;
    let head_width = fragment.width - peeled_width;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "offsets within one fragment are small; f64 is for accumulation only"
    )]
    let tail_glyphs: Vec<parley::Glyph> = fragment.glyphs[head_len..]
        .iter()
        .map(|glyph| parley::Glyph {
            x: (f64::from(glyph.x) - head_width) as f32,
            ..*glyph
        })
        .collect();
    Some((
        ParleyFragment {
            run: fragment.run,
            glyphs: fragment.glyphs[..head_len].to_vec(),
            width: head_width,
        },
        ParleyFragment {
            run: fragment.run,
            glyphs: tail_glyphs,
            width: peeled_width,
        },
    ))
}

/// The width of the trailing whitespace Blink would hang off the end of `line`, or
/// `None` if it would not hang any. See rule 2 in [`fragments_of`].
fn hanging_whitespace(line: &Line<'_, ()>, layout: &Layout<()>, bounds: &[usize]) -> Option<f64> {
    let width = f64::from(line.metrics().trailing_whitespace);
    let soft_broken = matches!(
        line.break_reason(),
        BreakReason::Regular | BreakReason::Emergency
    );
    let splits_a_span = !bounds.contains(&line.text_range().end);
    let is_last = lines_with_glyphs(layout)
        .last()
        .is_some_and(|last| last.text_range() == line.text_range());

    (width > 0.0 && soft_broken && splits_a_span && !is_last).then_some(width)
}

/// The lines of `layout` that actually paint something.
///
/// A line with no glyph runs has nothing to compare and no fragment to place, and
/// Parley produces one whenever it soft-breaks after the text's final space — which
/// Blink does not.
fn lines_with_glyphs(layout: &Layout<()>) -> impl DoubleEndedIterator<Item = Line<'_, ()>> {
    layout.lines().filter(|line| {
        line.items()
            .any(|item| matches!(item, PositionedLayoutItem::GlyphRun(_)))
    })
}

/// Where `line`'s content starts along the inline axis, taken from its first glyph run.
fn line_start(line: &Line<'_, ()>) -> f32 {
    line.items()
        .find_map(|item| match item {
            PositionedLayoutItem::GlyphRun(glyph_run) => Some(glyph_run.offset()),
            PositionedLayoutItem::InlineBox(_) => None,
        })
        .unwrap_or(0.0)
}

/// The case's span boundaries, as byte offsets into the concatenated text, including
/// both ends.
fn span_bounds(case: &Case) -> Vec<usize> {
    let mut bounds = Vec::with_capacity(case.runs.len() + 1);
    bounds.push(0);
    let mut offset = 0;
    for run in &case.runs {
        offset += run.text.len();
        bounds.push(offset);
    }
    bounds
}

/// Which span `offset` falls in, given [`span_bounds`].
fn span_of(offset: usize, bounds: &[usize]) -> usize {
    bounds
        .partition_point(|&bound| bound <= offset)
        .saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use parley_glyph_positioning_cases::Case;

    use super::*;

    #[test]
    fn smoke() {
        let mut font_cx = font_context();
        let mut layout_cx = LayoutContext::new();
        for seed in 0..64 {
            let case = Case::from_seed(seed);
            let laid_out = layout(&case, &mut font_cx, &mut layout_cx);
            let output = parley_output(&laid_out, &case);

            assert!(output.glyph_count() > 0, "seed {seed}: produced no glyphs");
            assert!(
                !output.styles.is_empty() && output.styles.len() <= case.runs.len(),
                "seed {seed}: expected 1..={} styles (v1 has one font, so styles are \
                 deduplicated purely on font size), got {}",
                case.runs.len(),
                output.styles.len()
            );
            for style in &output.styles {
                assert_eq!(
                    style.postscript_name, "Roboto-Regular",
                    "seed {seed}: unexpected PostScript name"
                );
            }
            for fragment in &output.fragments {
                assert!(
                    (fragment.style as usize) < output.styles.len(),
                    "seed {seed}: fragment references out-of-range style {}",
                    fragment.style
                );
                assert!(
                    !fragment.glyphs.is_empty(),
                    "seed {seed}: an empty fragment survived `build`"
                );
            }

            // Re-running from the same `Case` must reproduce identical output.
            let mut layout_cx_2 = LayoutContext::new();
            let laid_out_2 = layout(&case, &mut font_cx, &mut layout_cx_2);
            let output_2 = parley_output(&laid_out_2, &case);
            assert_eq!(
                output, output_2,
                "seed {seed}: extraction is not deterministic"
            );
        }
    }

    /// Every fragment origin after the first on a line must land on the 1/64 px grid,
    /// and no fragment may start to the left of where its unrounded position would put
    /// it — the snap is a ceiling, never a round.
    #[test]
    fn fragment_origins_are_ceiled_onto_the_layout_unit_grid() {
        let mut font_cx = font_context();
        let mut layout_cx = LayoutContext::new();
        let mut checked = 0;
        for seed in 0..64 {
            let case = Case::from_seed(seed);
            let laid_out = layout(&case, &mut font_cx, &mut layout_cx);
            let output = parley_output(&laid_out, &case);

            let mut previous: Option<(f64, f64)> = None;
            for fragment in &output.fragments {
                if let Some((origin_y, end)) = previous {
                    // Same baseline means the same line, so this fragment follows the
                    // previous one rather than starting a fresh line at its own edge.
                    if origin_y == fragment.origin_y {
                        let snapped = fragment.origin_x * 64.0;
                        assert!(
                            (snapped - snapped.round()).abs() < 1e-4,
                            "seed {seed}: fragment origin {} is off the 1/64 px grid",
                            fragment.origin_x
                        );
                        assert!(
                            fragment.origin_x >= end,
                            "seed {seed}: fragment origin {} precedes the unrounded end \
                             {end} of the fragment before it; the snap must be a ceiling",
                            fragment.origin_x
                        );
                        checked += 1;
                    }
                }
                let width: f64 = fragment
                    .glyphs
                    .last()
                    .map_or(0.0, |glyph| fragment.origin_x + f64::from(glyph.x));
                previous = Some((fragment.origin_y, width));
            }
        }
        assert!(
            checked > 0,
            "no case produced a second fragment on a line, so nothing was checked"
        );
    }
}
