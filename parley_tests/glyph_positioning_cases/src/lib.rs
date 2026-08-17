// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Test cases and golden output for comparing Parley with Chromium.

mod chromium_quantization;
mod compare;
mod freetype_hack;
mod generate;
mod glyph_output;
mod harness_css;
mod minimise;
mod signature;
mod typeface;

pub use chromium_quantization::{
    LAYOUT_UNIT_STEPS_PER_PX, MAX_ADVANCE_EPSILON, SPACING_GRID_STEPS_PER_PX, ceil_to_layout_unit,
    floor_to_layout_unit, half_ulp_6sig, position_tolerance, quantize_font_size, x_matches,
    y_matches,
};
pub use compare::{GlyphDiff, Mismatch, compare};
pub use freetype_hack::hack_would_fire;
pub use generate::{Case, MAX_CASE_WIDTH_PX, Run, alphabet, valid_font_size};
pub use glyph_output::{
    Fragment, GlyphOutput, GlyphOutputBuilder, Golden, LocalGlyph, ParseGoldenError,
    PositionedGlyph, Style,
};
pub use harness_css::{container_css, run_css};
pub use minimise::{
    MinimiseError, MinimiseOutcome, MinimisePhaseStats, MinimiseStats, Oracle, OracleActivity,
    OracleFailure, case_content_key, minimise,
};
pub use signature::FailureSignature;
pub use typeface::{TypefaceError, postscript_name_from_bytes, scan_for_valid_sfnts};

/// A font available to the test harness.
#[derive(Clone, Copy, Debug)]
pub struct SupportedFont {
    /// CSS family name.
    pub family: &'static str,
    /// Font file contents.
    pub bytes: &'static [u8],
    /// Stable download location for the font.
    pub url: &'static str,
    /// SHA-256 digest of the font, in lowercase hexadecimal.
    pub sha256: &'static str,
}

/// Fonts used by the test cases.
pub const FONTS: &[SupportedFont] = &[SupportedFont {
    family: "Roboto",
    bytes: include_bytes!("../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf"),
    url: "https://cdn.jsdelivr.net/gh/linebender/parley@a0752c7/parley_dev/assets/fonts/\
          roboto_fonts/Roboto-Regular.ttf",
    sha256: "319cff6e7a31f0f2a41c475dca42890aa5d19fe16017e2290f8c1d4e14f76481",
}];
