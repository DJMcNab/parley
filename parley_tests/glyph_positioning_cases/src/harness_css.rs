// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Mapping a [`Case`] to the CSS declaration strings a browser applies for it.
//!
//! The browser side knows nothing about the [`Case`]/[`Run`] grammar — it applies these
//! opaque declaration strings. That keeps the grammar in one typed, testable language:
//! adding a style property in a later version touches only this file, not an unlinted
//! script. See `doc/glyph-positioning-chrome-parity-phase2.md`.
//!
//! This lives here rather than in the recorder crate because the report generator
//! (`parley_glyph_positioning_report`) reproduces the recorder's DOM in a plain HTML
//! page, and must apply *byte-identical* CSS to be worth looking at; duplicating these
//! strings is exactly the drift the Phase 1 crate split exists to prevent. The recorder
//! remains the only caller that turns them into a `WebDriver` payload.

use std::fmt::Write as _;

use crate::FONTS;
use crate::generate::{Case, Run};

/// The CSS declarations applied to the container element for `case`.
#[must_use]
pub fn container_css(case: &Case) -> String {
    format!("width:{}px", case.width)
}

/// The CSS declarations applied to `run`'s span.
///
/// The font size is written **untruncated**: Blink truncates it to 1/100px in its font
/// cache, and the Parley side applies that truncation itself, so writing the raw value
/// here exercises the quantization rather than assuming it. Generated sizes sit at
/// least 0.0005px clear of a hundredth boundary, so the shortest-round-trip decimal
/// written here cannot truncate to a different hundredth than the Parley side computes.
#[must_use]
pub fn run_css(run: &Run) -> String {
    // v1's grammar carries no font family (there is exactly one font), so every run
    // uses the sole registered family.
    let mut css = format!("font-family:\"{}\"", FONTS[0].family);
    write!(css, ";font-size:{}px", run.font_size).unwrap();
    write!(css, ";letter-spacing:{}px", run.letter_spacing).unwrap();
    write!(css, ";word-spacing:{}px", run.word_spacing).unwrap();
    // An absolute length, not a unitless number or `normal` — see `Run::line_height`
    // for why.
    write!(css, ";line-height:{}px", run.line_height).unwrap();
    css
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_css_is_stable() {
        let run = Run {
            text: "unused".to_string(),
            font_size: 17.2341,
            letter_spacing: 0.5,
            word_spacing: -0.25,
            line_height: 24.984375,
        };
        assert_eq!(
            run_css(&run),
            "font-family:\"Roboto\";font-size:17.2341px;letter-spacing:0.5px;word-spacing:-0.25px;\
             line-height:24.984375px",
            "the CSS the page applies changed; goldens were recorded against the old form"
        );
    }

    #[test]
    fn container_css_is_stable() {
        let mut case = Case::from_seed(0);
        case.width = 312.5;
        assert_eq!(container_css(&case), "width:312.5px");
    }
}
