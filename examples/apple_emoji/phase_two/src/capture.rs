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
