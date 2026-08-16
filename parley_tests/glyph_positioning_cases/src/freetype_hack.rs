// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Reports whether Chromium's `FreeType` ascent/descent adjustment applies.
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
