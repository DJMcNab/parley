// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Chromium quantization behaviours that both case generation and the Parley-side
//! extraction need to model, plus the comparison predicate that follows from them.
//!
//! See "Chromium behaviour modelled" and "Comparison predicate" in
//! `doc/glyph-positioning-chrome-parity-phase1.md`.

/// Blink's font-size cache key (`kFontSizePrecisionMultiplier`): font sizes are
/// truncated to 1/100 CSS px before use, so e.g. `16.789` and `16.783` render
/// identically and `16.79`/`16.80` collide.
///
/// This models only that truncation. Blink *also* quantizes the size to `LayoutUnit`'s
/// 1/64 px grid, whose rounding mode is not established — so cases sample sizes on the
/// 1/4 px grid where both are exact (see `FONT_SIZE_STEP`), which leaves this a no-op for
/// every generated case and keeps the 1/64 grid out of the comparison entirely. A
/// handwritten case that picks a size off that grid is choosing to reintroduce an
/// unmodelled error term.
pub fn quantize_font_size(size: f32) -> f32 {
    (size * 100.0).floor() / 100.0
}

/// The number of steps per CSS px in Blink's `LayoutUnit` grid.
pub const LAYOUT_UNIT_STEPS_PER_PX: f32 = 64.0;

/// Floors `value` down to the nearest multiple of 1/64 CSS px (Blink's `LayoutUnit`),
/// so it survives that fixed-point conversion losslessly.
pub fn floor_to_layout_unit(value: f32) -> f32 {
    (value * LAYOUT_UNIT_STEPS_PER_PX).floor() / LAYOUT_UNIT_STEPS_PER_PX
}

/// Rounds `value` **up** to the nearest multiple of 1/64 CSS px, modelling Blink's
/// `LayoutUnit::FromFloatCeil`.
///
/// This is what places every fragment after the first on a line:
/// `ShapeResult::SnappedWidth()` and `ShapeResultView::SnappedWidth()`
/// (`third_party/blink/renderer/platform/fonts/shaping/{shape_result.h:170,
/// shape_result_view.h:124}`) are both `LayoutUnit::FromFloatCeil(width_)` — an
/// explicit ceiling, distinct from the plain `LayoutUnit(float)` constructor used
/// elsewhere in Blink (which truncates toward zero) and from `FromFloatRound`. So a
/// fragment's origin is always at or to the right of its unrounded position, never to
/// the left.
///
/// Accepts and returns f64 because the Parley side accumulates positions in f64.
pub fn ceil_to_layout_unit(value: f64) -> f64 {
    (value * f64::from(LAYOUT_UNIT_STEPS_PER_PX)).ceil() / f64::from(LAYOUT_UNIT_STEPS_PER_PX)
}

/// The number of steps per CSS px used when *sampling* letter/word spacing.
///
/// This is coarser than Blink's actual `TextRunLayoutUnit` grid (1/65536 px); see
/// "Spacing" in the Phase 1 doc for why sampling on this coarser grid is deliberate —
/// it sidesteps needing to know that conversion's rounding mode.
pub const SPACING_GRID_STEPS_PER_PX: f32 = 256.0;

/// The margin added to a case's `width` before asking Parley to break lines.
///
/// This is Blink's `AvailableWidthToFit` epsilon (1/64 px) plus a second 1/64 px for
/// the 16.16-vs-f32 advance-accumulation slack, applied unconditionally rather than
/// only on failure. See "Chromium behaviour modelled" in the Phase 1 doc for why.
pub const MAX_ADVANCE_EPSILON: f32 = 2.0 / LAYOUT_UNIT_STEPS_PER_PX;

/// Half a ULP at 6 significant figures — `skp_parser`'s serialisation precision, and
/// therefore the noise floor for any comparison against its output.
///
/// See "Comparison predicate — Term 1" in the Phase 1 doc.
pub fn half_ulp_6sig(value: f64) -> f64 {
    if value == 0.0 {
        // The half-ulp at 6 significant figures for a value of magnitude ~1.
        return 5e-6;
    }
    let magnitude = value.abs().log10().floor();
    10_f64.powf(magnitude - 5.0) / 2.0
}

/// Half a ULP at 6 significant figures for a *recorded* position, which `skp_parser`
/// writes as two independently-rounded numbers: the fragment's origin and the glyph's
/// offset from it.
///
/// The comparison works with their sum, so its error floor is the sum of their two
/// half-ULPs — not [`half_ulp_6sig`] of the sum. While a case had at most one fragment
/// per line every origin was 0 and the two agreed, which is why this only became
/// load-bearing once a line could hold several fragments.
pub fn position_tolerance(origin: f64, offset: f32) -> f64 {
    half_ulp_6sig(origin) + half_ulp_6sig(f64::from(offset))
}

/// Returns whether Parley's absolute `parley_x` (accumulated in f64; see the Phase 1
/// doc's "Parley must accumulate in f64") matches the Chrome position recorded as
/// `chrome_origin_x + chrome_offset_x`:
///
/// `|parley_x − (origin + offset)| ≤ position_tolerance(origin, offset)`
///
/// **Phase 4 correction**: this used to carry a second, `index_in_line`-scaled
/// `2⁻¹⁶` term tolerating Blink accumulating advances in 16.16 fixed point. Parley
/// now quantizes advances to that same grid (see
/// `doc/glyph-positioning-16-16-advances.md`), so both sides accumulate identically and
/// only `skp_parser`'s serialisation separates them. Dropping the term also drops the
/// need to know which line a glyph is on — which was never soundly derivable, since a
/// combining mark's GPOS y-offset means it does not share its base glyph's `y`.
pub fn x_matches(parley_x: f64, chrome_origin_x: f64, chrome_offset_x: f32) -> bool {
    let chrome_x = chrome_origin_x + f64::from(chrome_offset_x);
    (parley_x - chrome_x).abs() <= position_tolerance(chrome_origin_x, chrome_offset_x)
}

/// Returns whether `value` lands on a grid with `steps_per_px` steps per CSS px, to
/// floating-point tolerance.
///
/// The tolerance (`1e-3` grid steps) is far tighter than the spacing between any two of
/// this crate's grids (1/4, 1/64, 1/256 px), so it only forgives the float error from
/// constructing a value as `round(x * steps_per_px) / steps_per_px` — never a value that
/// is genuinely off-grid. Used by [`crate::generate::valid_font_size`] and the
/// minimiser's candidate-validity check (`src/minimise.rs`).
pub(crate) fn is_on_grid(value: f32, steps_per_px: f32) -> bool {
    let steps = value * steps_per_px;
    (steps - steps.round()).abs() < 1e-3
}

/// Returns whether Parley's absolute `parley_y` matches the Chrome position recorded as
/// `chrome_origin_y + chrome_offset_y`. Same two-part serialisation as [`x_matches`].
pub fn y_matches(parley_y: f64, chrome_origin_y: f64, chrome_offset_y: f32) -> bool {
    let chrome_y = chrome_origin_y + f64::from(chrome_offset_y);
    (parley_y - chrome_y).abs() <= position_tolerance(chrome_origin_y, chrome_offset_y)
}
