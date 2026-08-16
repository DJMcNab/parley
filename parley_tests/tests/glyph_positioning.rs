// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Compares Parley with checked-in Chromium glyph-positioning snapshots.
//!
//! Run with `cargo test -p parley_tests --test tests glyph_positioning`.
//! Snapshots in `known_failing` must continue to fail; move one to `regressions` when
//! its underlying bug is fixed.

use std::path::{Path, PathBuf};

use parley::{FontContext, LayoutContext};
use parley_glyph_positioning_cases::{Golden, compare};
use parley_glyph_positioning_extract::{font_context, layout, parley_output};

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
    let result = compare(&parley_output(&laid_out, &golden.case), &golden.output);

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
