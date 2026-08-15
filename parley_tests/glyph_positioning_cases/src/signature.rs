// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A coarse, deliberately lossy projection of a [`Mismatch`], used by the failure
//! minimiser (`src/minimise.rs`) to check that a shrunk candidate still fails "the same
//! way" as the case it was shrunk from.
//!
//! A [`FailureSignature`] keeps only the variant (`GlyphCount` vs `Glyphs`) and, for
//! `Glyphs`, the `same_multiset` flag plus which axes drift — a handful of coarse
//! buckets. It deliberately excludes fragile detail (diff counts, magnitudes, indices,
//! glyph ids): comparing on those would make minimisation slide from the bug it started
//! shrinking into an unrelated one, or flip a `Glyphs` failure into `GlyphCount` via an
//! incidental line-count change, rather than shrinking the original failure.

use crate::compare::Mismatch;

/// A coarse classification of a [`Mismatch`]. See the module docs for why it is this
/// coarse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureSignature {
    /// Parley and Chrome produced different numbers of glyphs.
    GlyphCount,
    /// Parley and Chrome produced the same number of glyphs, but at least one pair
    /// disagreed.
    Glyphs {
        /// Whether re-pairing both sides by position instead of by emission order
        /// removes the mismatch — see [`Mismatch::Glyphs`]'s `same_multiset`.
        same_multiset: bool,
        /// Whether any diff's `dx` exceeds its `x_tolerance`.
        x_drift: bool,
        /// Whether any diff's `dy` exceeds its `y_tolerance`.
        y_drift: bool,
    },
}

impl FailureSignature {
    /// Projects a [`Mismatch`] down to its [`FailureSignature`].
    #[must_use]
    pub fn of(mismatch: &Mismatch) -> Self {
        match mismatch {
            Mismatch::GlyphCount { .. } => Self::GlyphCount,
            Mismatch::Glyphs {
                diffs,
                same_multiset,
                ..
            } => Self::Glyphs {
                same_multiset: *same_multiset,
                // `dx`/`dy` are f64 (Parley accumulates x in f64; see the Phase 1 doc's
                // "Parley must accumulate in f64"), while the tolerances are f32 — widen
                // the tolerance rather than narrow the diff, so the comparison happens
                // at the diff's own precision.
                x_drift: diffs
                    .iter()
                    .any(|diff| diff.dx.abs() > f64::from(diff.x_tolerance)),
                y_drift: diffs
                    .iter()
                    .any(|diff| diff.dy.abs() > f64::from(diff.y_tolerance)),
            },
        }
    }
}

impl std::fmt::Display for FailureSignature {
    /// Formats as a single token with no spaces or newlines, so it can sit inline in a
    /// one-line note field (see `Golden::note`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlyphCount => f.write_str("glyph-count"),
            Self::Glyphs {
                same_multiset,
                x_drift,
                y_drift,
            } => {
                write!(
                    f,
                    "glyphs(multiset={}",
                    if *same_multiset { "same" } else { "different" }
                )?;
                if *x_drift {
                    f.write_str(",dx")?;
                }
                if *y_drift {
                    f.write_str(",dy")?;
                }
                f.write_str(")")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::GlyphDiff;
    use crate::glyph_output::{PositionedGlyph, Style};

    /// Builds a `GlyphDiff` with the given `dx`/`dy`/tolerances; every other field is an
    /// arbitrary fixed value, since [`FailureSignature::of`] only looks at those four.
    fn diff(dx: f64, dy: f64, x_tolerance: f32, y_tolerance: f32) -> GlyphDiff {
        let style = Style {
            postscript_name: "Roboto-Regular".to_string(),
            font_size: 16.0,
        };
        let glyph = PositionedGlyph {
            id: 1,
            x: 0.0,
            y: 0.0,
            style: 0,
        };
        GlyphDiff {
            index: 0,
            parley: glyph,
            chrome: glyph,
            parley_style: style.clone(),
            chrome_style: style,
            dx,
            dy,
            x_tolerance,
            y_tolerance,
        }
    }

    #[test]
    fn glyph_count_projects_to_glyph_count() {
        let mismatch = Mismatch::GlyphCount {
            parley: 3,
            chrome: 4,
        };
        assert_eq!(
            FailureSignature::of(&mismatch),
            FailureSignature::GlyphCount,
            "a glyph-count mismatch must project to the GlyphCount signature"
        );
    }

    #[test]
    fn glyphs_with_no_drift_projects_within_tolerance() {
        let mismatch = Mismatch::Glyphs {
            diffs: vec![diff(0.4, 0.4, 1.0, 1.0)],
            total: 2,
            same_multiset: true,
        };
        assert_eq!(
            FailureSignature::of(&mismatch),
            FailureSignature::Glyphs {
                same_multiset: true,
                x_drift: false,
                y_drift: false,
            },
            "diffs within tolerance must not register as drift"
        );
    }

    #[test]
    fn glyphs_with_x_drift_only() {
        let mismatch = Mismatch::Glyphs {
            diffs: vec![diff(2.0, 0.0, 1.0, 1.0)],
            total: 2,
            same_multiset: false,
        };
        assert_eq!(
            FailureSignature::of(&mismatch),
            FailureSignature::Glyphs {
                same_multiset: false,
                x_drift: true,
                y_drift: false,
            },
            "a diff whose |dx| exceeds x_tolerance must register as x_drift only"
        );
    }

    #[test]
    fn glyphs_with_both_axes_drifting_across_different_diffs() {
        let mismatch = Mismatch::Glyphs {
            diffs: vec![diff(2.0, 0.0, 1.0, 1.0), diff(0.0, 2.0, 1.0, 1.0)],
            total: 3,
            same_multiset: false,
        };
        assert_eq!(
            FailureSignature::of(&mismatch),
            FailureSignature::Glyphs {
                same_multiset: false,
                x_drift: true,
                y_drift: true,
            },
            "drift on either axis, in any diff, must set that axis's flag"
        );
    }

    #[test]
    fn display_has_no_whitespace() {
        let signatures = [
            FailureSignature::GlyphCount,
            FailureSignature::Glyphs {
                same_multiset: true,
                x_drift: false,
                y_drift: false,
            },
            FailureSignature::Glyphs {
                same_multiset: false,
                x_drift: true,
                y_drift: true,
            },
        ];
        for signature in signatures {
            let rendered = signature.to_string();
            assert!(
                !rendered.contains(char::is_whitespace),
                "signature Display must be a single token with no whitespace, got {rendered:?}"
            );
        }
    }

    #[test]
    fn display_is_stable_and_distinct() {
        assert_eq!(FailureSignature::GlyphCount.to_string(), "glyph-count");
        assert_eq!(
            FailureSignature::Glyphs {
                same_multiset: true,
                x_drift: false,
                y_drift: false,
            }
            .to_string(),
            "glyphs(multiset=same)"
        );
        assert_eq!(
            FailureSignature::Glyphs {
                same_multiset: false,
                x_drift: true,
                y_drift: true,
            }
            .to_string(),
            "glyphs(multiset=different,dx,dy)"
        );
    }
}
