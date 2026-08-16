// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

/// Truncates a font size to Chromium's 1/100 CSS pixel precision.
pub fn quantize_font_size(size: f32) -> f32 {
    (size * 100.0).floor() / 100.0
}

/// Number of Chromium layout units per CSS pixel.
pub const LAYOUT_UNIT_STEPS_PER_PX: f32 = 64.0;

/// Floors a value to Chromium's layout-unit grid.
pub fn floor_to_layout_unit(value: f32) -> f32 {
    (value * LAYOUT_UNIT_STEPS_PER_PX).floor() / LAYOUT_UNIT_STEPS_PER_PX
}

/// Ceils a value to Chromium's layout-unit grid.
pub fn ceil_to_layout_unit(value: f64) -> f64 {
    (value * f64::from(LAYOUT_UNIT_STEPS_PER_PX)).ceil() / f64::from(LAYOUT_UNIT_STEPS_PER_PX)
}

/// Number of sampling-grid steps per CSS pixel for text spacing.
pub const SPACING_GRID_STEPS_PER_PX: f32 = 256.0;

/// Extra width allowed when reproducing Chromium line breaks.
///
/// One layout unit matches Chromium's epsilon; the other covers advance accumulation.
pub const MAX_ADVANCE_EPSILON: f32 = 2.0 / LAYOUT_UNIT_STEPS_PER_PX;

/// Returns half a unit in the last place at six significant figures.
pub fn half_ulp_6sig(value: f64) -> f64 {
    if value == 0.0 {
        // Recorded zeroes have no magnitude, so use the precision around 1.
        return 5e-6;
    }
    let magnitude = value.abs().log10().floor();
    10_f64.powf(magnitude - 5.0) / 2.0
}

/// Returns the serialization tolerance for a separately recorded origin and offset.
pub fn position_tolerance(origin: f64, offset: f32) -> f64 {
    half_ulp_6sig(origin) + half_ulp_6sig(f64::from(offset))
}

/// Tests an x position against Chromium's recorded origin and offset.
pub fn x_matches(parley_x: f64, chrome_origin_x: f64, chrome_offset_x: f32) -> bool {
    let chrome_x = chrome_origin_x + f64::from(chrome_offset_x);
    (parley_x - chrome_x).abs() <= position_tolerance(chrome_origin_x, chrome_offset_x)
}

pub(crate) fn is_on_grid(value: f32, steps_per_px: f32) -> bool {
    let steps = value * steps_per_px;
    (steps - steps.round()).abs() < 1e-3
}

/// Tests a y position against Chromium's recorded origin and offset.
pub fn y_matches(parley_y: f64, chrome_origin_y: f64, chrome_offset_y: f32) -> bool {
    let chrome_y = chrome_origin_y + f64::from(chrome_offset_y);
    (parley_y - chrome_y).abs() <= position_tolerance(chrome_origin_y, chrome_offset_y)
}
