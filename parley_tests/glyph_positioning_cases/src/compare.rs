// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Diffing Parley's [`GlyphOutput`] against Chrome's.
//!
//! This lives here rather than in the recorder crate because the CI test needs the same
//! comparison and must not depend on the recorder; duplicating it is exactly the drift
//! the Phase 1 crate split exists to prevent. See "Comparison" in
//! `doc/glyph-positioning-chrome-parity-phase4.md`.

use crate::chromium_quantization::{half_ulp_6sig, x_matches, y_matches};
use crate::glyph_output::{GlyphOutput, PositionedGlyph, Style};

/// How many diffs [`Mismatch`]'s [`std::fmt::Display`] prints before truncating.
const MAX_REPORTED_DIFFS: usize = 10;

/// Compares Parley's output against Chrome's, glyph by glyph in emission order.
///
/// Glyphs pair up by index: Phase 0 confirmed Blink paints blobs in reading order, which
/// is Parley's order too. That holds only because v1 has no bidi — Phase 1 established
/// the bundled font has no `R`/`AL` codepoints and asserts it — so neither the pairing
/// nor the emission order should be assumed to survive bidi support.
///
/// Styles are compared as resolved `(postscript_name, font_size)` pairs, never by index
/// into the two (independently built) style tables.
pub fn compare(parley: &GlyphOutput, chrome: &GlyphOutput) -> Result<(), Mismatch> {
    if parley.glyphs.len() != chrome.glyphs.len() {
        return Err(Mismatch::GlyphCount {
            parley: parley.glyphs.len(),
            chrome: chrome.glyphs.len(),
        });
    }

    let diffs = diff(parley, &parley.glyphs, chrome, &chrome.glyphs);
    if diffs.is_empty() {
        return Ok(());
    }

    // Failure path only: re-pair by position instead of by emission order, so a
    // paint-order difference reports as one line rather than as a wall of position
    // diffs. This hides nothing — it runs only once the ordered comparison has failed.
    let parley_sorted = sorted(&parley.glyphs);
    let chrome_sorted = sorted(&chrome.glyphs);
    let same_multiset = diff(parley, &parley_sorted, chrome, &chrome_sorted).is_empty();

    Err(Mismatch::Glyphs {
        diffs,
        total: parley.glyphs.len(),
        same_multiset,
    })
}

/// Compares two equal-length glyph sequences index by index, resolving each glyph's
/// style against its owning [`GlyphOutput`].
fn diff(
    parley: &GlyphOutput,
    parley_glyphs: &[PositionedGlyph],
    chrome: &GlyphOutput,
    chrome_glyphs: &[PositionedGlyph],
) -> Vec<GlyphDiff> {
    parley_glyphs
        .iter()
        .zip(chrome_glyphs)
        .enumerate()
        .filter_map(|(index, (&parley_glyph, &chrome_glyph))| {
            let parley_style = style_of(parley, parley_glyph);
            let chrome_style = style_of(chrome, chrome_glyph);
            let matches = parley_glyph.id == chrome_glyph.id
                && parley_style == chrome_style
                && x_matches(f64::from(parley_glyph.x), chrome_glyph.x)
                && y_matches(parley_glyph.y, chrome_glyph.y);
            (!matches).then(|| GlyphDiff {
                index,
                parley: parley_glyph,
                chrome: chrome_glyph,
                parley_style: parley_style.clone(),
                chrome_style: chrome_style.clone(),
                dx: f64::from(parley_glyph.x) - f64::from(chrome_glyph.x),
                dy: f64::from(parley_glyph.y) - f64::from(chrome_glyph.y),
                x_tolerance: half_ulp_6sig(chrome_glyph.x),
                y_tolerance: half_ulp_6sig(chrome_glyph.y),
            })
        })
        .collect()
}

/// Resolves a glyph's style index against its owning output's style table.
fn style_of(output: &GlyphOutput, glyph: PositionedGlyph) -> &Style {
    output
        .styles
        .get(usize::from(glyph.style))
        .expect("a glyph's style index must be in range for its own output")
}

/// Orders glyphs by `(y, x, id)`, for the failure path's paint-order-insensitive
/// re-check.
fn sorted(glyphs: &[PositionedGlyph]) -> Vec<PositionedGlyph> {
    let mut sorted = glyphs.to_vec();
    sorted.sort_by(|a, b| {
        a.y.total_cmp(&b.y)
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| a.id.cmp(&b.id))
    });
    sorted
}

/// How Parley's output differed from Chrome's.
///
/// This carries no case identity — [`GlyphOutput`] does not know which case produced it
/// — so callers prefix their own.
#[derive(Clone, Debug)]
pub enum Mismatch {
    /// The two sides produced different numbers of glyphs, so no pairing is meaningful.
    GlyphCount {
        /// How many glyphs Parley produced.
        parley: usize,
        /// How many glyphs Chrome produced.
        chrome: usize,
    },
    /// The two sides produced the same number of glyphs, but some pair disagreed.
    Glyphs {
        /// Every disagreeing pair, in emission order.
        diffs: Vec<GlyphDiff>,
        /// How many glyphs were compared in total.
        total: usize,
        /// Whether re-pairing both sides by `(y, x, id)` instead of by emission order
        /// makes the mismatch disappear — i.e. the same glyphs were painted in a
        /// different order.
        same_multiset: bool,
    },
}

/// One disagreeing glyph pair.
#[derive(Clone, Debug)]
pub struct GlyphDiff {
    /// The pair's index in emission order.
    pub index: usize,
    /// Parley's glyph.
    pub parley: PositionedGlyph,
    /// Chrome's glyph.
    pub chrome: PositionedGlyph,
    /// Parley's glyph's resolved style.
    pub parley_style: Style,
    /// Chrome's glyph's resolved style.
    pub chrome_style: Style,
    /// `parley.x − chrome.x`.
    pub dx: f64,
    /// `parley.y − chrome.y`.
    pub dy: f64,
    /// The tolerance [`dx`](Self::dx) was allowed.
    pub x_tolerance: f32,
    /// The tolerance [`dy`](Self::dy) was allowed.
    pub y_tolerance: f32,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlyphCount { parley, chrome } => {
                write!(f, "glyph count differs: parley {parley}, chrome {chrome}")
            }
            Self::Glyphs {
                diffs,
                total,
                same_multiset,
            } => {
                writeln!(f, "{} of {total} glyphs differ", diffs.len())?;
                if *same_multiset {
                    writeln!(
                        f,
                        "same glyphs, different order: re-pairing both sides by (y, x, id) \
                         matches, so this is a paint-order difference rather than a \
                         positioning one"
                    )?;
                }
                for diff in diffs.iter().take(MAX_REPORTED_DIFFS) {
                    writeln!(f, "{diff}")?;
                }
                if diffs.len() > MAX_REPORTED_DIFFS {
                    writeln!(f, "... and {} more", diffs.len() - MAX_REPORTED_DIFFS)?;
                }
                Ok(())
            }
        }
    }
}

impl std::fmt::Display for GlyphDiff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            index,
            parley,
            chrome,
            parley_style,
            chrome_style,
            dx,
            dy,
            x_tolerance,
            y_tolerance,
        } = self;
        write!(
            f,
            "  [{index}] parley: id {} at ({}, {}) {} {}px | chrome: id {} at ({}, {}) {} {}px \
             | dx {dx:+.6} (tol {x_tolerance:.6}) dy {dy:+.6} (tol {y_tolerance:.6})",
            parley.id,
            parley.x,
            parley.y,
            parley_style.postscript_name,
            parley_style.font_size,
            chrome.id,
            chrome.x,
            chrome.y,
            chrome_style.postscript_name,
            chrome_style.font_size,
        )
    }
}

impl std::error::Error for Mismatch {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyph_output::GlyphOutputBuilder;

    /// Builds a `GlyphOutput` from `(id, x, y, font_size)` tuples, all in one font.
    fn output(glyphs: &[(u32, f32, f32, f32)]) -> GlyphOutput {
        let mut builder = GlyphOutputBuilder::default();
        for &(id, x, y, font_size) in glyphs {
            let style = builder.style_index(Style {
                postscript_name: "Roboto-Regular".to_string(),
                font_size,
            });
            builder.push_glyph(id, x, y, style);
        }
        builder.build()
    }

    #[test]
    fn identical_output_matches() {
        let a = output(&[(1, 0.0, 22.0, 24.0), (2, 17.1094, 22.0, 24.0)]);
        assert!(compare(&a, &a).is_ok());
    }

    #[test]
    fn within_the_serialisation_floor_matches() {
        // `skp_parser` emits 6 significant figures, so 192.609 stands for anything in
        // 192.6085..192.6095.
        let parley = output(&[(1, 192.60948, 22.0, 24.0)]);
        let chrome = output(&[(1, 192.609, 22.0, 24.0)]);
        assert!(compare(&parley, &chrome).is_ok());
    }

    #[test]
    fn just_past_the_serialisation_floor_differs() {
        let parley = output(&[(1, 192.6105, 22.0, 24.0)]);
        let chrome = output(&[(1, 192.609, 22.0, 24.0)]);
        let Err(Mismatch::Glyphs { diffs, total, .. }) = compare(&parley, &chrome) else {
            panic!("expected a glyph-level mismatch");
        };
        assert_eq!((diffs.len(), total), (1, 1));
    }

    #[test]
    fn a_differing_style_is_a_mismatch_even_at_the_same_position() {
        let parley = output(&[(1, 10.0, 22.0, 24.0)]);
        let chrome = output(&[(1, 10.0, 22.0, 23.99)]);
        assert!(compare(&parley, &chrome).is_err());
    }

    /// Style tables are built independently on each side, so equal indices need not mean
    /// equal styles — only the resolved `(name, size)` pair may be compared.
    #[test]
    fn styles_compare_by_value_not_by_index() {
        let parley = output(&[(1, 0.0, 22.0, 12.0), (2, 10.0, 22.0, 24.0)]);
        let chrome = output(&[(1, 0.0, 22.0, 12.0), (2, 10.0, 22.0, 24.0)]);
        assert_eq!(parley.styles.len(), 2);
        assert!(compare(&parley, &chrome).is_ok());

        // The same glyphs, but with the style table in the opposite order.
        let mut swapped = chrome.clone();
        swapped.styles.reverse();
        for glyph in &mut swapped.glyphs {
            glyph.style = 1 - glyph.style;
        }
        assert!(compare(&parley, &swapped).is_ok());
    }

    #[test]
    fn differing_counts_report_as_counts() {
        let parley = output(&[(1, 0.0, 22.0, 24.0)]);
        let chrome = output(&[(1, 0.0, 22.0, 24.0), (2, 10.0, 22.0, 24.0)]);
        let Err(mismatch @ Mismatch::GlyphCount { .. }) = compare(&parley, &chrome) else {
            panic!("expected a count mismatch");
        };
        assert_eq!(
            mismatch.to_string(),
            "glyph count differs: parley 1, chrome 2"
        );
    }

    #[test]
    fn a_reordering_reports_as_one() {
        let parley = output(&[(1, 0.0, 22.0, 24.0), (2, 10.0, 22.0, 24.0)]);
        let chrome = output(&[(2, 10.0, 22.0, 24.0), (1, 0.0, 22.0, 24.0)]);
        let Err(mismatch) = compare(&parley, &chrome) else {
            panic!("expected a mismatch");
        };
        let Mismatch::Glyphs { same_multiset, .. } = &mismatch else {
            panic!("expected a glyph-level mismatch");
        };
        assert!(same_multiset);
        assert!(
            mismatch
                .to_string()
                .contains("same glyphs, different order")
        );
    }

    /// A genuine positioning failure must not be excused as a reordering.
    #[test]
    fn a_real_position_difference_is_not_a_reordering() {
        let parley = output(&[(1, 0.0, 22.0, 24.0), (2, 10.0, 22.0, 24.0)]);
        let chrome = output(&[(1, 0.0, 22.0, 24.0), (2, 11.0, 22.0, 24.0)]);
        let Err(Mismatch::Glyphs { same_multiset, .. }) = compare(&parley, &chrome) else {
            panic!("expected a glyph-level mismatch");
        };
        assert!(!same_multiset);
    }

    #[test]
    fn display_truncates_a_long_diff_list() {
        let parley: Vec<_> = (0..25).map(|i| (i, i as f32, 22.0, 24.0)).collect();
        let chrome: Vec<_> = (0..25).map(|i| (i, i as f32 + 1.0, 22.0, 24.0)).collect();
        let Err(mismatch) = compare(&output(&parley), &output(&chrome)) else {
            panic!("expected a mismatch");
        };
        let report = mismatch.to_string();
        assert!(report.starts_with("25 of 25 glyphs differ"));
        assert!(report.contains("... and 15 more"));
    }
}
