// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::compare::Mismatch;

/// The mismatch properties preserved while minimising a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureSignature {
    /// Glyph counts differ.
    GlyphCount,
    /// Fragment shapes differ.
    Fragmentation,
    /// Corresponding glyphs differ.
    Glyphs {
        /// Whether sorting by position makes the outputs equal.
        same_multiset: bool,
        /// Whether any x difference exceeds its tolerance.
        x_drift: bool,
        /// Whether any y difference exceeds its tolerance.
        y_drift: bool,
    },
}

impl FailureSignature {
    /// Extracts a signature from a mismatch.
    #[must_use]
    pub fn of(mismatch: &Mismatch) -> Self {
        match mismatch {
            Mismatch::GlyphCount { .. } => Self::GlyphCount,
            Mismatch::Fragmentation { .. } => Self::Fragmentation,
            Mismatch::Glyphs {
                diffs,
                same_multiset,
                ..
            } => Self::Glyphs {
                same_multiset: *same_multiset,
                x_drift: diffs.iter().any(|diff| diff.dx.abs() > diff.x_tolerance),
                y_drift: diffs.iter().any(|diff| diff.dy.abs() > diff.y_tolerance),
            },
        }
    }
}

impl std::fmt::Display for FailureSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GlyphCount => f.write_str("glyph-count"),
            Self::Fragmentation => f.write_str("fragmentation"),
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

    fn diff(dx: f64, dy: f64, x_tolerance: f64, y_tolerance: f64) -> GlyphDiff {
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
    fn fragmentation_projects_to_fragmentation() {
        let mismatch = Mismatch::Fragmentation {
            parley: vec![5],
            chrome: vec![2, 3],
        };
        assert_eq!(
            FailureSignature::of(&mismatch),
            FailureSignature::Fragmentation,
            "a fragmentation mismatch must project to its own signature, not to Glyphs"
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
            FailureSignature::Fragmentation,
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
        assert_eq!(FailureSignature::Fragmentation.to_string(), "fragmentation");
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
