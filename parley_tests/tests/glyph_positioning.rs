// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Checks that Parley's glyph positions match Chrome's, against the checked-in golden
//! corpus under `tests/glyph_positioning/`.
//!
//! No Docker, network, or `chromedriver` dependency: every golden file already holds
//! both the [`Case`](parley_glyph_positioning_cases::Case) that produced it and
//! Chrome's recorded output, so this test only needs to lay the case out with Parley
//! and compare. See `doc/glyph-positioning-chrome-parity.md` and its Phase 5 doc.
//!
//! Four directories are walked:
//!
//! - `handwritten/`, `regressions/`, `generated/` — expected to **pass**. A generated
//!   case must have been curated to avoid every known bug (see the Phase 5 doc for
//!   how the initial corpus was built); a failure here is a real regression.
//! - `known_failing/` — expected to **still fail**. These are checked-in repros of
//!   real, tracked bugs (each file's `note` says which). Asserting the failure rather
//!   than skipping the case means a fix shows up as a test failure telling you to
//!   promote the case out of `known_failing/`, instead of it going unnoticed.
//!
//! Every case in every directory runs regardless of earlier failures, and all
//! unexpected results are reported together at the end, so a CI run surfaces every
//! broken case at once rather than only the first one alphabetically.

use std::path::{Path, PathBuf};

use parley::{FontContext, LayoutContext};
use parley_glyph_positioning_cases::{Golden, compare};
use parley_glyph_positioning_extract::{font_context, layout, parley_output};

/// `tests/glyph_positioning/`, the checked-in golden corpus.
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/glyph_positioning")
}

#[test]
fn glyph_positioning_matches_chrome() {
    let mut font_cx = font_context();
    let mut layout_cx = LayoutContext::new();
    let root = corpus_root();

    let mut failures = Vec::new();
    for dir in ["handwritten", "regressions", "generated"] {
        for path in golden_files(&root, dir) {
            if let Some(failure) = check(&path, &mut font_cx, &mut layout_cx, false) {
                failures.push(failure);
            }
        }
    }
    for path in golden_files(&root, "known_failing") {
        if let Some(failure) = check(&path, &mut font_cx, &mut layout_cx, true) {
            failures.push(failure);
        }
    }

    assert!(
        failures.is_empty(),
        "{} case(s) did not have the expected result:\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

/// Lays out `path`'s case with Parley and compares it against the golden's recorded
/// Chrome output, returning `Some(message)` if the result wasn't the one expected for
/// its directory (`expect_failure` is true only for `known_failing/`).
fn check(
    path: &Path,
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    expect_failure: bool,
) -> Option<String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: failed to read: {error}", path.display()));
    let golden = Golden::parse(&text)
        .unwrap_or_else(|error| panic!("{}: failed to parse: {error}", path.display()));

    let laid_out = layout(&golden.case, font_cx, layout_cx);
    let result = compare(&parley_output(&laid_out), &golden.output);

    match (expect_failure, result) {
        (false, Ok(())) | (true, Err(_)) => None,
        (false, Err(mismatch)) => Some(format!("{}:\n{mismatch}", path.display())),
        (true, Ok(())) => Some(format!(
            "{}: listed in known_failing/ but now passes against Parley — promote it \
             (move the file into generated/ or regressions/, per what caused it)",
            path.display()
        )),
    }
}

/// Every `*.txt` golden file directly under `root/dir`, sorted. A missing directory
/// yields no files rather than an error, since `handwritten/` and `regressions/` may
/// be empty.
fn golden_files(root: &Path, dir: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
        return Vec::new();
    };
    let mut files: Vec<_> = entries
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .collect();
    files.sort();
    files
}
