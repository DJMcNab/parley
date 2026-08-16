// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Diffing Parley's [`GlyphOutput`] against Chrome's.
//!
//! This lives here rather than in the recorder crate because the CI test needs the same
//! comparison and must not depend on the recorder; duplicating it is exactly the drift
//! the Phase 1 crate split exists to prevent. See "Comparison" in
//! `doc/glyph-positioning-chrome-parity-phase4.md`.

use crate::chromium_quantization::{position_tolerance, x_matches, y_matches};
use crate::glyph_output::{Fragment, GlyphOutput, PositionedGlyph, Style};

/// How many diffs [`Mismatch`]'s [`std::fmt::Display`] prints before truncating.
const MAX_REPORTED_DIFFS: usize = 10;

/// Compares Parley's output against Chrome's, in three stages: total glyph count, then
/// fragmentation, then glyph by glyph in emission order.
///
/// The fragmentation stage exists because Parley derives its fragments — one span's
/// glyphs on one line — from its own layout, while Chrome's are recorded from what
/// Blink actually painted. When the two disagree, every position after the divergence
/// is wrong for one shared reason, and saying so once is far more useful than a wall of
/// position diffs. It is also the only way the one divergence Parley provably cannot
/// model surfaces as a diagnosis: two adjacent spans with identical styles are two
/// fragments to Blink but a single item to Parley, whose runs are split on resolved
/// style, not on span identity.
///
/// Glyphs pair up by index: Phase 0 confirmed Blink paints blobs in reading order, which
/// is Parley's order too. That holds only because v1 has no bidi — Phase 1 established
/// the bundled font has no `R`/`AL` codepoints and asserts it — so neither the pairing
/// nor the emission order should be assumed to survive bidi support.
///
/// Styles are compared as resolved `(postscript_name, font_size)` pairs, never by index
/// into the two (independently built) style tables.
pub fn compare(parley: &GlyphOutput, chrome: &GlyphOutput) -> Result<(), Mismatch> {
    if parley.glyph_count() != chrome.glyph_count() {
        return Err(Mismatch::GlyphCount {
            parley: parley.glyph_count(),
            chrome: chrome.glyph_count(),
        });
    }

    if parley.shape() != chrome.shape() {
        return Err(Mismatch::Fragmentation {
            parley: parley.shape(),
            chrome: chrome.shape(),
        });
    }

    let parley_glyphs = parley.glyphs();
    let chrome_glyphs = chrome.glyphs();
    let chrome_recorded = recorded(chrome);

    let diffs = diff(
        parley,
        &parley_glyphs,
        chrome,
        &chrome_glyphs,
        &chrome_recorded,
    );
    if diffs.is_empty() {
        return Ok(());
    }

    // Failure path only: re-pair by position instead of by emission order, so a
    // paint-order difference reports as one line rather than as a wall of position
    // diffs. This hides nothing — it runs only once the ordered comparison has failed.
    // Both sides are sorted together with Chrome's origins, so each Chrome glyph keeps
    // the origin its tolerance is computed from.
    let parley_sorted = sorted(&parley_glyphs, &recorded(parley));
    let chrome_sorted = sorted(&chrome_glyphs, &chrome_recorded);
    let same_multiset = diff(
        parley,
        &parley_sorted.0,
        chrome,
        &chrome_sorted.0,
        &chrome_sorted.1,
    )
    .is_empty();

    Err(Mismatch::Glyphs {
        diffs,
        total: parley_glyphs.len(),
        same_multiset,
    })
}

/// One glyph's position in the two parts it was recorded as, rather than as their sum.
///
/// `skp_parser` rounds the fragment origin and the glyph's offset from it to 6
/// significant figures *independently*, so both are needed to work out how far apart
/// the two sides are allowed to be — see [`position_tolerance`].
#[derive(Clone, Copy, Debug)]
struct Recorded {
    origin_x: f64,
    origin_y: f64,
    offset_x: f32,
    offset_y: f32,
}

/// Every glyph's [`Recorded`] position, in the same order as [`GlyphOutput::glyphs`].
fn recorded(output: &GlyphOutput) -> Vec<Recorded> {
    output
        .fragments
        .iter()
        .flat_map(|fragment: &Fragment| {
            fragment.glyphs.iter().map(move |glyph| Recorded {
                origin_x: fragment.origin_x,
                origin_y: fragment.origin_y,
                offset_x: glyph.x,
                offset_y: glyph.y,
            })
        })
        .collect()
}

/// Compares two equal-length glyph sequences index by index, resolving each glyph's
/// style against its owning [`GlyphOutput`].
///
/// `chrome_recorded` carries the two parts each Chrome glyph's position was recorded
/// as, which is what its serialisation tolerance is computed from — see
/// [`position_tolerance`].
fn diff(
    parley: &GlyphOutput,
    parley_glyphs: &[PositionedGlyph],
    chrome: &GlyphOutput,
    chrome_glyphs: &[PositionedGlyph],
    chrome_recorded: &[Recorded],
) -> Vec<GlyphDiff> {
    parley_glyphs
        .iter()
        .zip(chrome_glyphs)
        .zip(chrome_recorded)
        .enumerate()
        .filter_map(|(index, ((&parley_glyph, &chrome_glyph), &position))| {
            let parley_style = style_of(parley, parley_glyph);
            let chrome_style = style_of(chrome, chrome_glyph);
            let matches = parley_glyph.id == chrome_glyph.id
                && parley_style == chrome_style
                && x_matches(parley_glyph.x, position.origin_x, position.offset_x)
                && y_matches(parley_glyph.y, position.origin_y, position.offset_y);
            (!matches).then(|| GlyphDiff {
                index,
                parley: parley_glyph,
                chrome: chrome_glyph,
                parley_style: parley_style.clone(),
                chrome_style: chrome_style.clone(),
                dx: parley_glyph.x - chrome_glyph.x,
                dy: parley_glyph.y - chrome_glyph.y,
                x_tolerance: position_tolerance(position.origin_x, position.offset_x),
                y_tolerance: position_tolerance(position.origin_y, position.offset_y),
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
/// re-check, carrying each glyph's recorded position along with it.
fn sorted(
    glyphs: &[PositionedGlyph],
    recorded: &[Recorded],
) -> (Vec<PositionedGlyph>, Vec<Recorded>) {
    let mut paired: Vec<_> = glyphs
        .iter()
        .copied()
        .zip(recorded.iter().copied())
        .collect();
    paired.sort_by(|(a, _), (b, _)| {
        a.y.total_cmp(&b.y)
            .then_with(|| a.x.total_cmp(&b.x))
            .then_with(|| a.id.cmp(&b.id))
    });
    paired.into_iter().unzip()
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
    /// The two sides produced the same glyphs but split them into fragments
    /// differently, so no position comparison would mean anything.
    Fragmentation {
        /// Parley's glyph count per fragment, in order.
        parley: Vec<usize>,
        /// Chrome's glyph count per fragment, in order.
        chrome: Vec<usize>,
    },
    /// The two sides agreed on glyph count and fragmentation, but some pair disagreed.
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
    pub x_tolerance: f64,
    /// The tolerance [`dy`](Self::dy) was allowed.
    pub y_tolerance: f64,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlyphCount { parley, chrome } => {
                write!(f, "glyph count differs: parley {parley}, chrome {chrome}")
            }
            Self::Fragmentation { parley, chrome } => {
                write!(
                    f,
                    "the same {} glyphs were split into different fragments: parley \
                     {parley:?}, chrome {chrome:?} (glyphs per fragment). Every position \
                     after the first divergence follows from this, so none is reported. \
                     Known causes, in rough order of likelihood: the two sides broke the \
                     line somewhere different, so the same glyphs fall on different \
                     lines; a wrapped line's trailing whitespace hung on one side and \
                     not the other; or two adjacent spans resolved to identical styles, \
                     which Blink keeps as two fragments and snaps between while Parley's \
                     runs, split on resolved style, merge into one — the last of these \
                     cannot be modelled from Parley's side and is designed out of \
                     generated cases",
                    parley.iter().sum::<usize>(),
                )
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

    /// Builds a single-fragment `GlyphOutput` at origin 0 from `(id, x, y, font_size)`
    /// tuples. A style change starts a new fragment, matching the one-span-per-style
    /// shape the harness generates.
    fn output(glyphs: &[(u32, f32, f32, f32)]) -> GlyphOutput {
        let mut builder = GlyphOutputBuilder::default();
        let mut current = None;
        for &(id, x, y, font_size) in glyphs {
            let style = builder.style_index(Style {
                postscript_name: "Roboto-Regular".to_string(),
                font_size,
            });
            if current != Some(style) {
                builder.begin_fragment(0.0, 0.0, style);
                current = Some(style);
            }
            builder.push_glyph(id, x, y);
        }
        builder.build()
    }

    /// Builds a two-fragment `GlyphOutput`, both at `font_size`, with the second at
    /// `origin_x`. Glyph positions are offsets from their own fragment's origin.
    fn two_fragments(
        font_size: f32,
        first: &[(u32, f32)],
        origin_x: f64,
        second: &[(u32, f32)],
    ) -> GlyphOutput {
        let mut builder = GlyphOutputBuilder::default();
        let style = builder.style_index(Style {
            postscript_name: "Roboto-Regular".to_string(),
            font_size,
        });
        for (origin, glyphs) in [(0.0, first), (origin_x, second)] {
            builder.begin_fragment(origin, 0.0, style);
            for &(id, x) in glyphs {
                builder.push_glyph(id, x, 0.0);
            }
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

    /// The floor is the sum of the origin's and the offset's half-ULPs, so a glyph deep
    /// into a line — where the origin is large — is allowed more slack than the same
    /// absolute position measured from origin 0 would be.
    #[test]
    fn the_floor_accounts_for_the_fragment_origin() {
        // Origin 153.938 (half-ULP 5e-4) plus offset 11.1189 (half-ULP 5e-5) gives a
        // 5.5e-4 floor, where the absolute position 165.0569 alone would give 5e-4.
        let chrome = two_fragments(24.0, &[(1, 0.0)], 153.938, &[(2, 11.1189)]);
        let within = two_fragments(24.0, &[(1, 0.0)], 153.938, &[(2, 11.11942)]);
        let beyond = two_fragments(24.0, &[(1, 0.0)], 153.938, &[(2, 11.1196)]);
        assert!(
            compare(&within, &chrome).is_ok(),
            "a 5.2e-4 difference is inside the 5.5e-4 two-part floor"
        );
        assert!(
            compare(&beyond, &chrome).is_err(),
            "a 7.0e-4 difference is outside it"
        );
        // The very same difference at the very same absolute position, but recorded
        // from origin 0, gets only the one half-ULP and so is not forgiven.
        let flat_chrome = output(&[(1, 0.0, 0.0, 24.0), (2, 165.0569, 0.0, 24.0)]);
        let flat_within = output(&[(1, 0.0, 0.0, 24.0), (2, 165.05742, 0.0, 24.0)]);
        assert!(compare(&flat_within, &flat_chrome).is_err());
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
        for fragment in &mut swapped.fragments {
            fragment.style = 1 - fragment.style;
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

    /// The same glyphs split differently must report as one fragmentation mismatch, not
    /// as a wall of positions — this is how the adjacent-identical-styles divergence
    /// surfaces.
    #[test]
    fn differing_fragmentation_reports_as_fragmentation() {
        let parley = output(&[(1, 0.0, 0.0, 24.0), (2, 10.0, 0.0, 24.0)]);
        let chrome = two_fragments(24.0, &[(1, 0.0)], 10.0, &[(2, 0.0)]);
        let Err(mismatch @ Mismatch::Fragmentation { .. }) = compare(&parley, &chrome) else {
            panic!("expected a fragmentation mismatch");
        };
        let report = mismatch.to_string();
        assert!(report.contains("parley [2], chrome [1, 1]"), "got {report}");
    }

    /// Fragmentation is only reported when the shapes genuinely differ: the same shape
    /// with a differing origin is a position mismatch, since that is exactly the
    /// snapping the harness exists to check.
    #[test]
    fn matching_fragmentation_with_a_shifted_origin_is_a_position_mismatch() {
        let parley = two_fragments(24.0, &[(1, 0.0)], 10.0, &[(2, 0.0)]);
        let chrome = two_fragments(24.0, &[(1, 0.0)], 10.015625, &[(2, 0.0)]);
        let Err(Mismatch::Glyphs { diffs, .. }) = compare(&parley, &chrome) else {
            panic!("expected a glyph-level mismatch");
        };
        assert_eq!(diffs.len(), 1);
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
