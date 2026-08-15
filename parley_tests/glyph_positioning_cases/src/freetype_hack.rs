// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Blink's Linux/ChromeOS/Android/Fuchsia-only `FreeType` ascent/descent workaround.

/// Returns whether Blink's `FreeType` ascent/descent workaround
/// (`FontMetrics::AscentDescentWithHacks` in `font_metrics.cc`) would fire for a font
/// with hhea descender `hhea_descender` and `units_per_em`, rendered at
/// `css_font_size` CSS px.
///
/// Confirmed verbatim against Chrome-for-Testing tag `151.0.7922.77`:
///
/// ```cpp
/// // Linux / ChromeOS / Android / Fuchsia only
/// if (use_subpixel_positioning && descent < SkScalarToFloat(metrics.fDescent) && ascent >= 1) {
///   ++descent;  --ascent;
/// }
/// ```
///
/// `descent < fDescent` after rounding means the descent's fractional part in
/// physical px lies in `(0, 0.5)`. The `ascent >= 1` half of the condition isn't
/// modeled here: it always holds for fonts and sizes in the sampled 10-30px range
/// (for the bundled Roboto, `ascent_px` already exceeds 1 at the minimum sampled
/// size) — see "`FreeType` hack pre-filter" in
/// `doc/glyph-positioning-chrome-parity-phase1.md`.
pub fn hack_would_fire(hhea_descender: i16, units_per_em: u16, css_font_size: f32) -> bool {
    let descent_px = (hhea_descender as f32).abs() * css_font_size / units_per_em as f32;
    let frac = descent_px.fract();
    frac > 0.0 && frac < 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roboto_fires_for_roughly_half_the_range() {
        // Roboto: hhea descender -512, units_per_em 2048 (descent = size * 0.25).
        // At every whole-px size the fraction is exactly 0.0 or 0.5, i.e. never
        // strictly inside (0, 0.5); at in-between sizes it varies.
        assert!(
            !hack_would_fire(-512, 2048, 10.0),
            "exact half-px descent never fires (boundary excluded)"
        );
        assert!(
            hack_would_fire(-512, 2048, 8.8),
            "descent 2.2px (frac 0.2) should land in (0, 0.5)"
        );
    }
}
