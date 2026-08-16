// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded random generation of glyph-positioning Chrome-parity test cases, plus the
//! shared golden-output schema and Chromium-quantization helpers used to compare
//! against it.
//!
//! This crate deliberately has no dependency on `parley`: it exists so the test-case
//! grammar and golden schema can be shared, without drift, between the Parley-side
//! extraction (`parley_glyph_positioning_extract`) and the Chrome-side recorder, which
//! is native-only and excluded from the wasm/android CI matrix.
//!
//! See `doc/glyph-positioning-chrome-parity-phase1.md` for the full design; this crate
//! implements everything that file specifies for `parley_glyph_positioning_cases`.

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
    MinimiseError, MinimiseOutcome, Oracle, OracleFailure, case_content_key, minimise,
};
pub use signature::FailureSignature;
pub use typeface::{TypefaceError, postscript_name_from_bytes, scan_for_valid_sfnts};

/// A font the harness collects glyph-positioning data for.
#[derive(Clone, Copy, Debug)]
pub struct SupportedFont {
    /// The family name for the font, as passed to Parley and set in the harness's CSS.
    pub family: &'static str,
    /// The raw font file bytes, embedded so the browser harness needs no network or
    /// filesystem access.
    pub bytes: &'static [u8],
    /// Where [`Self::bytes`] can be fetched from, for consumers that can't embed them.
    ///
    /// This is the font's canonical download location. Most fonts are expected to stop
    /// being checked into this repository — the corpus needs more faces than it is
    /// reasonable to vendor — at which point this field is how they are obtained in the
    /// first place, and [`Self::sha256`] is what pins what was obtained. Until then,
    /// vendored faces name a commit-pinned URL for the copy in this repository, so the
    /// two fields mean the same thing for every entry.
    ///
    /// The URL must be served with a permissive `access-control-allow-origin`: the
    /// report generator's pages fetch it from a `file://` origin, which is opaque.
    pub url: &'static str,
    /// The lowercase hex SHA-256 of [`Self::bytes`], pinning what [`Self::url`] serves.
    ///
    /// `@font-face` cannot carry a subresource-integrity hash, so a page that loads the
    /// face over the network verifies it itself: fetch, digest, and only then construct
    /// the `FontFace`. Without this, a page's live-DOM rendering could silently use
    /// different bytes than the ones its images were rasterised from, which would read
    /// as a Parley bug rather than as the font mismatch it is.
    pub sha256: &'static str,
}

/// The fonts used by the glyph-positioning Chrome-parity harness.
///
/// v1 has exactly one entry. Every font here must have a distinct PostScript name (see
/// [`postscript_name_from_bytes`]), and this crate's [`Case`] generation always samples
/// against `FONTS[0]`'s `cmap` — multi-font support is out of scope for v1, but the
/// schema and this registry are shaped so it's an extension rather than a rewrite.
pub const FONTS: &[SupportedFont] = &[SupportedFont {
    family: "Roboto",
    bytes: include_bytes!("../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf"),
    // This face is vendored, so its URL is a commit-pinned view of the vendored copy.
    // The commit is pinned (not a branch) so the bytes behind the URL can never change.
    url: "https://cdn.jsdelivr.net/gh/linebender/parley@a0752c7/parley_dev/assets/fonts/\
          roboto_fonts/Roboto-Regular.ttf",
    sha256: "319cff6e7a31f0f2a41c475dca42890aa5d19fe16017e2290f8c1d4e14f76481",
}];
