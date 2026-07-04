// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Emoji measurement + pixel capture, adapted from
//! `examples/apple_emoji/phase_one/src/main.rs` (copied, not imported — `phase_one` must
//! not be edited or depended on).

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, ImageData, TextMetrics};

/// Margin (px) left around the measured ink box so nothing touches the canvas edge.
const PAD: f64 = 8.0;

/// Everything captured for a single emoji at a given font size.
#[allow(dead_code, reason = "fbb_ascent kept for capture symmetry with earlier phases; unused by phase_four's top-anchored baseline placement (§6.3)")]
pub(crate) struct EmojiCapture {
    pub advance_width: f64,
    pub abb_ascent: f64,
    pub abb_descent: f64,
    pub fbb_ascent: f64,
    pub canvas_width: u32,
    pub canvas_height: u32,
    /// Column in the capture buffer corresponding to the text pen-origin x.
    pub origin_x: f64,
    /// Row in the capture buffer corresponding to the alphabetic baseline.
    pub origin_y: f64,
    /// Straight (non-premultiplied) RGBA8 pixels, `canvas_width * canvas_height * 4` bytes.
    pub pixels: Vec<u8>,
}

/// Measure + rasterize `emoji` at `font_size` (CSS px) using a scratch canvas, returning
/// the metrics and captured pixel buffer. Uses the same technique proven in Phase One.
///
/// Note: headless Chrome's `TextMetrics.actualBoundingBox*` for system color emoji has
/// been observed to fall back to a constant (`font_size + 3`-ish) rather than a real ink
/// measurement (see `examples/apple_emoji/ink_survey/noto_analysis.txt`), so those fields
/// are measurement-only leftovers here and must not be used for sizing/centering — use
/// [`pixel_ink_bbox`] (a real per-pixel scan of the rasterized capture) instead.
pub(crate) fn capture_emoji(
    document: &Document,
    scratch_canvas: &HtmlCanvasElement,
    emoji: &str,
    font_size: f64,
) -> Result<EmojiCapture, JsValue> {
    let _ = document;
    let ctx = get_2d_context(scratch_canvas)?;

    let font = format!(
        "{font_size}px 'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji', sans-serif"
    );
    ctx.set_font(&font);
    ctx.set_text_baseline("alphabetic");

    let tm: TextMetrics = ctx.measure_text(emoji)?;
    let advance_width = tm.width();
    let abb_left = tm.actual_bounding_box_left();
    let abb_right = tm.actual_bounding_box_right();
    let abb_ascent = tm.actual_bounding_box_ascent();
    let abb_descent = tm.actual_bounding_box_descent();
    let fbb_ascent = tm.font_bounding_box_ascent();

    let width = (abb_left + abb_right + 2.0 * PAD).ceil().max(1.0);
    let height = (abb_ascent + abb_descent + 2.0 * PAD).ceil().max(1.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas dimensions are small, computed from finite emoji metrics"
    )]
    let canvas_width = width as u32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas dimensions are small, computed from finite emoji metrics"
    )]
    let canvas_height = height as u32;
    scratch_canvas.set_width(canvas_width);
    scratch_canvas.set_height(canvas_height);

    // Resizing the canvas resets 2D context state; re-apply.
    ctx.set_font(&font);
    ctx.set_text_baseline("alphabetic");

    let origin_x = PAD + abb_left;
    let origin_y = PAD + abb_ascent;
    ctx.fill_text(emoji, origin_x, origin_y)?;

    let image_data: ImageData =
        ctx.get_image_data(0.0, 0.0, f64::from(canvas_width), f64::from(canvas_height))?;
    let pixels: Vec<u8> = image_data.data().0;

    Ok(EmojiCapture {
        advance_width,
        abb_ascent,
        abb_descent,
        fbb_ascent,
        canvas_width,
        canvas_height,
        origin_x,
        origin_y,
        pixels,
    })
}

pub(crate) fn get_2d_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    Ok(canvas
        .get_context("2d")?
        .ok_or("no 2d context")?
        .dyn_into::<CanvasRenderingContext2d>()?)
}

/// A real, per-pixel ink bounding box of a [`EmojiCapture`]'s rasterized pixels (alpha >
/// 0), in capture-canvas pixel coordinates (inclusive on all edges). Used for horizontal
/// centering instead of `TextMetrics.actualBoundingBox*`, which is unreliable for system
/// color emoji in headless Chrome (see module docs on [`capture_emoji`]).
pub(crate) struct PixelInk {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl PixelInk {
    pub(crate) fn width(&self) -> f64 {
        self.right - self.left + 1.0
    }

    pub(crate) fn height(&self) -> f64 {
        self.bottom - self.top + 1.0
    }

    /// Horizontal center of the ink, in capture-canvas pixel coordinates.
    pub(crate) fn center_x(&self) -> f64 {
        (self.left + self.right + 1.0) / 2.0
    }
}

/// Scan `cap`'s rasterized pixels for the tightest bounding box of non-transparent
/// pixels (`alpha > 0`). Returns `None` if the capture is fully transparent (should not
/// happen for a real emoji glyph).
pub(crate) fn pixel_ink_bbox(cap: &EmojiCapture) -> Option<PixelInk> {
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (u32::MAX, 0_u32, u32::MAX, 0_u32);
    let mut found = false;
    for y in 0..cap.canvas_height {
        for x in 0..cap.canvas_width {
            let idx = ((y * cap.canvas_width + x) * 4 + 3) as usize;
            if cap.pixels[idx] > 0 {
                found = true;
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if !found {
        return None;
    }
    Some(PixelInk {
        left: f64::from(min_x),
        right: f64::from(max_x),
        top: f64::from(min_y),
        bottom: f64::from(max_y),
    })
}
