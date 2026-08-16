// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::chromium_quantization::{position_tolerance, x_matches, y_matches};
use crate::glyph_output::{Fragment, GlyphOutput, PositionedGlyph, Style};

const MAX_REPORTED_DIFFS: usize = 10;

/// Compares output shape, then pairs glyphs in emission order.
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

    // A second comparison distinguishes paint-order changes from changed glyphs.
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

#[derive(Clone, Copy, Debug)]
struct Recorded {
    origin_x: f64,
    origin_y: f64,
    offset_x: f32,
    offset_y: f32,
}

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

fn style_of(output: &GlyphOutput, glyph: PositionedGlyph) -> &Style {
    output
        .styles
        .get(usize::from(glyph.style))
        .expect("a glyph's style index must be in range for its own output")
}

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

/// A difference between Parley and Chromium output.
#[derive(Clone, Debug)]
pub enum Mismatch {
    /// The outputs contain different numbers of glyphs.
    GlyphCount {
        /// Number of Parley glyphs.
        parley: usize,
        /// Number of Chromium glyphs.
        chrome: usize,
    },
    /// The outputs divide glyphs into fragments differently.
    Fragmentation {
        /// Parley glyph counts by fragment.
        parley: Vec<usize>,
        /// Chromium glyph counts by fragment.
        chrome: Vec<usize>,
    },
    /// Corresponding glyphs differ.
    Glyphs {
        /// Differences in emission order.
        diffs: Vec<GlyphDiff>,
        /// Number of glyphs compared.
        total: usize,
        /// Whether position-sorted outputs contain the same glyphs.
        same_multiset: bool,
    },
}

/// Details of one differing glyph pair.
#[derive(Clone, Debug)]
pub struct GlyphDiff {
    /// Index in emission order.
    pub index: usize,
    /// Parley glyph.
    pub parley: PositionedGlyph,
    /// Chromium glyph.
    pub chrome: PositionedGlyph,
    /// Resolved Parley style.
    pub parley_style: Style,
    /// Resolved Chromium style.
    pub chrome_style: Style,
    /// Horizontal difference in CSS pixels.
    pub dx: f64,
    /// Vertical difference in CSS pixels.
    pub dy: f64,
    /// Permitted horizontal difference.
    pub x_tolerance: f64,
    /// Permitted vertical difference.
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
    fn the_floor_accounts_for_the_fragment_origin() {
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

    #[test]
    fn styles_compare_by_value_not_by_index() {
        let parley = output(&[(1, 0.0, 22.0, 12.0), (2, 10.0, 22.0, 24.0)]);
        let chrome = output(&[(1, 0.0, 22.0, 12.0), (2, 10.0, 22.0, 24.0)]);
        assert_eq!(parley.styles.len(), 2);
        assert!(compare(&parley, &chrome).is_ok());

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
