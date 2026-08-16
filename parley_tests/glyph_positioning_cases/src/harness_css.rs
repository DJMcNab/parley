// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;

use crate::FONTS;
use crate::generate::{Case, Run};

/// Builds the container declarations for a case.
#[must_use]
pub fn container_css(case: &Case) -> String {
    format!("width:{}px", case.width)
}

/// Builds the span declarations for a run.
#[must_use]
pub fn run_css(run: &Run) -> String {
    let mut css = format!("font-family:\"{}\"", FONTS[0].family);
    write!(css, ";font-size:{}px", run.font_size).unwrap();
    write!(css, ";letter-spacing:{}px", run.letter_spacing).unwrap();
    write!(css, ";word-spacing:{}px", run.word_spacing).unwrap();
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
