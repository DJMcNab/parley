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
    FontContext, FontFamily, Layout, LayoutContext, LineHeight, PositionedLayoutItem, StyleProperty,
};
use parley_glyph_positioning_cases::{
    Case, FONTS, GlyphOutput, GlyphOutputBuilder, MAX_ADVANCE_EPSILON, Style,
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

    let mut layout = builder.build(&text);
    layout.break_all_lines(Some(case.width + MAX_ADVANCE_EPSILON));
    layout
}

/// Extracts [`GlyphOutput`] from an already-broken `layout`.
///
/// Glyph `x` is accumulated in **f64**, not via [`GlyphRun::positioned_glyphs`] (whose
/// f32 running offset would itself contribute more error than the 16.16-accumulation
/// drift this harness measures against Chrome) — see "Parley must accumulate in f64"
/// in the Phase 1 doc. The per-glyph advances themselves stay f32; only the
/// accumulator changes.
///
/// [`GlyphRun::positioned_glyphs`]: parley::GlyphRun::positioned_glyphs
#[must_use]
pub fn parley_output(layout: &Layout<()>) -> GlyphOutput {
    let mut builder = GlyphOutputBuilder::default();
    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };

            let font = glyph_run.run().font();
            let postscript_name =
                postscript_name_from_bytes(font.font.data.data(), font.font.index)
                    .expect("Parley-selected font must have a readable PostScript name");
            let style_index = builder.style_index(Style {
                postscript_name,
                font_size: glyph_run.run().font_size(),
            });

            let baseline = f64::from(glyph_run.baseline());
            let mut x = f64::from(glyph_run.offset());
            for glyph in glyph_run.glyphs() {
                let gx = x + f64::from(glyph.x);
                let gy = baseline + f64::from(glyph.y);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "positions are well within f32 range; f64 was only needed for \
                              accumulation precision"
                )]
                builder.push_glyph(glyph.id, gx as f32, gy as f32, style_index);
                x += f64::from(glyph.advance);
            }
        }
    }
    builder.build()
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
            let output = parley_output(&laid_out);

            assert!(!output.glyphs.is_empty(), "seed {seed}: produced no glyphs");
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
            for glyph in &output.glyphs {
                assert!(
                    (glyph.style as usize) < output.styles.len(),
                    "seed {seed}: glyph references out-of-range style {}",
                    glyph.style
                );
            }

            // Re-running from the same `Case` must reproduce identical output.
            let mut layout_cx_2 = LayoutContext::new();
            let laid_out_2 = layout(&case, &mut font_cx, &mut layout_cx_2);
            let output_2 = parley_output(&laid_out_2);
            assert_eq!(
                output, output_2,
                "seed {seed}: extraction is not deterministic"
            );
        }
    }
}
