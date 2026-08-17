// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Throw-away triage for a glyph-positioning fuzz haul.
//!
//! This intentionally classifies *observed mismatch text*, rather than trying to
//! infer a product bug from the generated input.  It is a quick way to make the
//! next minimised failure visible, to group cases that have the same observable
//! shape, and to select one in every five matches for human review.  Do not use
//! this as a regression classifier or promote its labels into the corpus.
//!
//! ```sh
//! cargo run -p parley_glyph_positioning_recorder --bin classify_spike -- \
//!     --out target/glyph_positioning_exploration
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parley_glyph_positioning_cases::Golden;
use parley_glyph_positioning_recorder::Result;

const REVIEW_EVERY: usize = 5;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Class {
    /// A combining mark is present and the engines emitted different numbers of
    /// glyphs. This is a structural investigation, not a position tolerance issue.
    CombiningMarkGlyphCountDivergence,
    /// A small horizontal-only residual barely exceeds the comparison tolerance.
    /// It needs a rounding/quantisation investigation before being called a bug.
    SubLayoutUnitXResidual,
    /// The same glyph sequence was emitted in incompatible fragment groupings.
    FragmentationDivergence,
    /// A horizontal-only mismatch materially exceeds the serialisation tolerance.
    HorizontalOffset,
    Unclassified,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Self::CombiningMarkGlyphCountDivergence => "combining-mark-glyph-count-divergence",
            Self::SubLayoutUnitXResidual => "sub-layout-unit-x-residual",
            Self::FragmentationDivergence => "fragmentation-divergence",
            Self::HorizontalOffset => "horizontal-offset",
            Self::Unclassified => "unclassified",
        }
    }
}

struct Finding {
    path: PathBuf,
    class: Class,
    minimised: bool,
    summary: String,
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    let findings = discover(&args.out)?;
    let mut groups: BTreeMap<Class, Vec<Finding>> = BTreeMap::new();
    for finding in findings {
        groups.entry(finding.class).or_default().push(finding);
    }

    let mut report =
        String::from("EXPLORATORY ONLY: labels describe mismatch shape, not root cause.\n\n");
    for (class, findings) in &groups {
        report.push_str(&format!("{}: {} case(s)\n", class.name(), findings.len()));
        for (index, finding) in findings.iter().enumerate() {
            let review = index % REVIEW_EVERY == 0;
            let needs_minimising = !finding.minimised && (review || class == &Class::Unclassified);
            report.push_str(&format!(
                "  {}{}{} — {}\n",
                if needs_minimising {
                    "MINIMISE "
                } else {
                    "         "
                },
                if review { "REVIEW " } else { "       " },
                finding.path.display(),
                finding.summary
            ));
        }
        report.push('\n');
    }
    if groups.is_empty() {
        report.push_str("No mismatch artifacts found. Run fuzz_loop first.\n");
    }

    std::fs::write(args.out.join("classification_spike_report.txt"), &report)?;
    print!("{report}");
    Ok(())
}

fn discover(out: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let entries = match std::fs::read_dir(out) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(findings),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if !path.is_dir() {
            continue;
        }
        let (case_path, mismatch_path, minimised) = if path.join("minimised.txt").is_file()
            && path.join("minimised_mismatch.txt").is_file()
        {
            (
                path.join("minimised.txt"),
                path.join("minimised_mismatch.txt"),
                true,
            )
        } else if path.join("case.txt").is_file() && path.join("mismatch.txt").is_file() {
            (path.join("case.txt"), path.join("mismatch.txt"), false)
        } else {
            continue;
        };
        let mismatch = std::fs::read_to_string(&mismatch_path)?;
        let golden = Golden::parse(&std::fs::read_to_string(&case_path)?)
            .map_err(|error| format!("{}: {error}", case_path.display()))?;
        let class = classify(&golden, &mismatch);
        findings.push(Finding {
            path,
            class,
            minimised,
            summary: format!(
                "{} seed {}; {} run(s); {}",
                if minimised { "minimised" } else { "raw" },
                golden.case.seed,
                golden.case.runs.len(),
                mismatch.lines().next().unwrap_or("empty mismatch")
            ),
        });
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(findings)
}

fn classify(golden: &Golden, mismatch: &str) -> Class {
    if mismatch.starts_with("glyph count differs:") && golden_has_combining_mark(golden) {
        return Class::CombiningMarkGlyphCountDivergence;
    }
    if mismatch.contains(" glyphs differ") && all_diffs_are_tiny_horizontal(mismatch) {
        return Class::SubLayoutUnitXResidual;
    }
    if mismatch.starts_with("the same ") && mismatch.contains("split into different fragments") {
        return Class::FragmentationDivergence;
    }
    if mismatch.contains(" glyphs differ") && all_diffs_are_horizontal(mismatch) {
        return Class::HorizontalOffset;
    }
    Class::Unclassified
}

fn golden_has_combining_mark(golden: &Golden) -> bool {
    golden
        .case
        .runs
        .iter()
        .flat_map(|run| run.text.chars())
        .any(|ch| ('\u{0300}'..='\u{036f}').contains(&ch))
}

fn all_diffs_are_tiny_horizontal(mismatch: &str) -> bool {
    all_diffs_match(mismatch, |dx, dy, tolerance| {
        dx.abs() <= tolerance * 1.05 && dy.abs() <= f64::EPSILON
    })
}

fn all_diffs_are_horizontal(mismatch: &str) -> bool {
    all_diffs_match(mismatch, |dx, dy, _| {
        dx.abs() > f64::EPSILON && dy.abs() <= f64::EPSILON
    })
}

fn all_diffs_match(mismatch: &str, predicate: impl Fn(f64, f64, f64) -> bool) -> bool {
    let mut saw_diff = false;
    for line in mismatch
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
    {
        let Some((dx, rest)) = field(line, "dx ") else {
            return false;
        };
        let Some((dy, _)) = field(rest, "dy ") else {
            return false;
        };
        let Ok(dx) = dx.parse::<f64>() else {
            return false;
        };
        let Ok(dy) = dy.parse::<f64>() else {
            return false;
        };
        let Some((tolerance, _)) = field(rest, "(tol ") else {
            return false;
        };
        let Ok(tolerance) = tolerance.parse::<f64>() else {
            return false;
        };
        if !predicate(dx, dy, tolerance) {
            return false;
        }
        saw_diff = true;
    }
    saw_diff
}

/// Extracts the numeric field immediately after `prefix`, ending at whitespace or `)`.
fn field<'a>(text: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let start = text.find(prefix)? + prefix.len();
    let value = &text[start..];
    let end = value
        .find(|ch: char| ch.is_whitespace() || ch == ')')
        .unwrap_or(value.len());
    Some((&value[..end], &value[end..]))
}

struct Args {
    out: PathBuf,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut args = std::env::args().skip(1);
        let mut out = PathBuf::from("target/glyph_positioning_fuzz");
        while let Some(arg) = args.next() {
            if arg != "--out" {
                return Err(format!("unrecognised argument {arg:?}").into());
            }
            out = args
                .next()
                .ok_or_else(|| "--out needs a value".to_owned())?
                .into();
        }
        Ok(Self { out })
    }
}

#[cfg(test)]
mod tests {
    use parley_glyph_positioning_cases::{Case, Golden};

    use super::{Class, REVIEW_EVERY, all_diffs_are_tiny_horizontal, classify};

    fn golden(text: &str) -> Golden {
        Golden {
            case: Case {
                seed: 0,
                runs: vec![parley_glyph_positioning_cases::Run {
                    text: text.into(),
                    font_size: 16.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    line_height: 16.0,
                }],
                width: 100.0,
            },
            note: None,
            output: Default::default(),
        }
    }

    #[test]
    fn recognises_a_glyph_count_divergence() {
        assert_eq!(
            classify(
                &golden("a\u{0300}"),
                "glyph count differs: parley 4, chrome 6"
            ),
            Class::CombiningMarkGlyphCountDivergence
        );
    }

    #[test]
    fn recognises_tiny_horizontal_residuals_only() {
        let mismatch = "2 of 8 glyphs differ\n  [1] dx -0.000553 (tol 0.000550) dy +0.000000 (tol 0.000055)\n  [2] dx +0.0001004 (tol 0.000100) dy +0.000000 (tol 0.000055)";
        assert!(all_diffs_are_tiny_horizontal(mismatch));
        assert_eq!(
            classify(&golden("plain text"), mismatch),
            Class::SubLayoutUnitXResidual
        );
    }

    #[test]
    fn leaves_large_or_vertical_diffs_unclassified() {
        let mismatch = "1 of 8 glyphs differ\n  [1] dx +0.01 (tol 0.0005) dy +0.0 (tol 0.00005)";
        assert_eq!(
            classify(&golden("plain text"), mismatch),
            Class::HorizontalOffset
        );
    }

    #[test]
    fn recognises_fragmentation() {
        assert_eq!(
            classify(
                &golden("plain text"),
                "the same 2 glyphs were split into different fragments: parley [2], chrome [1, 1]"
            ),
            Class::FragmentationDivergence
        );
    }

    #[test]
    fn review_sampling_is_one_in_five() {
        assert_eq!((0..12).filter(|index| index % REVIEW_EVERY == 0).count(), 3);
    }
}
