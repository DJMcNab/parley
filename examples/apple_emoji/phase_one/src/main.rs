// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase One: prove that a Rust wasm app can obtain Apple's **system** color-emoji
//! glyph pixels *and* metrics, without embedding or serving the Apple emoji font.
//!
//! Approach (see `plans/phase_one_plan.md` for the full design):
//!
//! 1. Draw an emoji into a 2D canvas (`#source`) using `ctx.font = "128px 'Apple Color
//!    Emoji', ..."`.
//! 2. Read `TextMetrics` (advance width + bounding boxes) from `measureText`.
//! 3. `getImageData` the rasterized pixels into an owned Rust `Vec<u8>`.
//! 4. Analyze the pixels in Rust (color proof, alpha coverage, border-clip scan,
//!    pixel-derived ink box vs `TextMetrics` cross-check).
//! 5. Reconstruct a *second* canvas (`#roundtrip`) from the Rust-owned buffer via
//!    `putImageData`, with a Rust-drawn magenta frame around the detected ink box,
//!    proving Rust actually holds and can manipulate the pixel bytes.
//!
//! Build/run with [Trunk](https://trunkrs.dev):
//!
//! ```sh
//! rustup target add wasm32-unknown-unknown
//! cargo install --locked trunk
//! cd examples/apple_emoji/phase_one
//! trunk serve
//! ```

use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, Document, HtmlCanvasElement, ImageData, TextMetrics};

/// The emoji we use as the primary proof glyph: an unambiguous, fully-qualified,
/// single-codepoint colored emoji (U+1F600 GRINNING FACE).
const EMOJI: &str = "😀";

/// Font size used for the capture, in CSS px (== canvas backing-store px, since we
/// don't apply a `devicePixelRatio` transform).
const FONT_SIZE: f64 = 128.0;

/// Margin (px) left around the measured ink box so nothing touches the canvas edge.
const PAD: f64 = 16.0;

/// Threshold for "genuinely colored" (as opposed to grayscale) pixels: the max
/// pairwise channel difference among R, G, B must exceed this.
const COLOR_CHANNEL_DIFF_THRESHOLD: u8 = 12;

fn main() {
    console_error_panic_hook::set_once();
    wasm_bindgen_futures::spawn_local(async {
        if let Err(err) = run().await {
            web_sys::console::error_1(&err);
            if let Some(document) = web_sys::window().and_then(|w| w.document()) {
                let _ = set_status(&document, "FAILED — see console");
            }
        }
    });
}

/// All the metrics + analysis we report, both to the page and as JSON.
#[derive(Debug)]
struct Report {
    advance_width: f64,
    actual_bounding_box_left: f64,
    actual_bounding_box_right: f64,
    actual_bounding_box_ascent: f64,
    actual_bounding_box_descent: f64,
    font_bounding_box_ascent: f64,
    font_bounding_box_descent: f64,
    canvas_width: u32,
    canvas_height: u32,
    colored_pixel_count: u32,
    non_transparent_pixel_count: u32,
    alpha_coverage_percent: f64,
    border_clipped: bool,
    pixel_ink_box: Option<(u32, u32, u32, u32)>,
    ink_box_consistent_with_metrics: bool,
    pass: bool,
    fail_reasons: Vec<String>,
}

impl Report {
    fn to_display_string(&self) -> String {
        format!(
            "advance width (TextMetrics.width):      {:.3} px\n\
             actualBoundingBoxLeft:                  {:.3} px\n\
             actualBoundingBoxRight:                 {:.3} px\n\
             actualBoundingBoxAscent:                {:.3} px\n\
             actualBoundingBoxDescent:                {:.3} px\n\
             fontBoundingBoxAscent:                   {:.3} px\n\
             fontBoundingBoxDescent:                  {:.3} px\n\
             canvas size:                             {} x {} px\n\
             colored pixels (real color emoji proof): {}\n\
             non-transparent pixels:                  {}\n\
             alpha coverage:                          {:.2}%\n\
             border clipped:                          {}\n\
             pixel-derived ink box (x0,y0,x1,y1):     {:?}\n\
             ink box consistent with TextMetrics:     {}\n\
             \n\
             VERDICT: {}\n",
            self.advance_width,
            self.actual_bounding_box_left,
            self.actual_bounding_box_right,
            self.actual_bounding_box_ascent,
            self.actual_bounding_box_descent,
            self.font_bounding_box_ascent,
            self.font_bounding_box_descent,
            self.canvas_width,
            self.canvas_height,
            self.colored_pixel_count,
            self.non_transparent_pixel_count,
            self.alpha_coverage_percent,
            self.border_clipped,
            self.pixel_ink_box,
            self.ink_box_consistent_with_metrics,
            if self.pass {
                "PASS".to_owned()
            } else {
                format!("FAIL ({})", self.fail_reasons.join("; "))
            }
        )
    }

    fn to_json(&self) -> String {
        let ink_box = match self.pixel_ink_box {
            Some((x0, y0, x1, y1)) => format!(
                "{{\"x0\":{x0},\"y0\":{y0},\"x1\":{x1},\"y1\":{y1}}}"
            ),
            None => "null".to_owned(),
        };
        let fail_reasons = self
            .fail_reasons
            .iter()
            .map(|r| format!("{:?}", r))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"advance_width\":{},\"actual_bounding_box_left\":{},\"actual_bounding_box_right\":{},\
             \"actual_bounding_box_ascent\":{},\"actual_bounding_box_descent\":{},\
             \"font_bounding_box_ascent\":{},\"font_bounding_box_descent\":{},\
             \"canvas_width\":{},\"canvas_height\":{},\"colored_pixel_count\":{},\
             \"non_transparent_pixel_count\":{},\"alpha_coverage_percent\":{},\
             \"border_clipped\":{},\"pixel_ink_box\":{},\"ink_box_consistent_with_metrics\":{},\
             \"pass\":{},\"fail_reasons\":[{}]}}",
            self.advance_width,
            self.actual_bounding_box_left,
            self.actual_bounding_box_right,
            self.actual_bounding_box_ascent,
            self.actual_bounding_box_descent,
            self.font_bounding_box_ascent,
            self.font_bounding_box_descent,
            self.canvas_width,
            self.canvas_height,
            self.colored_pixel_count,
            self.non_transparent_pixel_count,
            self.alpha_coverage_percent,
            self.border_clipped,
            ink_box,
            self.ink_box_consistent_with_metrics,
            self.pass,
            fail_reasons,
        )
    }
}

async fn run() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    set_status(&document, "Waiting for fonts to be ready…")?;
    // Best-effort: system fonts should already be available, but guard against a
    // first-frame race in some browsers.
    if let Ok(ready) = document.fonts().ready() {
        let _ = JsFuture::from(ready).await;
    }
    yield_now(&window).await;

    set_status(&document, "Measuring…")?;

    let source_canvas = get_canvas(&document, "source")?;
    let source_ctx = get_2d_context(&source_canvas)?;

    let font = format!("{FONT_SIZE}px 'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji', sans-serif");
    source_ctx.set_font(&font);
    source_ctx.set_text_baseline("alphabetic");

    // First, measure on a 1x1 scratch-sized canvas context (font state carries over
    // canvas resize as long as we re-set it after resizing).
    let tm: TextMetrics = source_ctx.measure_text(EMOJI)?;
    let advance_width = tm.width();
    let abb_left = tm.actual_bounding_box_left();
    let abb_right = tm.actual_bounding_box_right();
    let abb_ascent = tm.actual_bounding_box_ascent();
    let abb_descent = tm.actual_bounding_box_descent();
    let fbb_ascent = tm.font_bounding_box_ascent();
    let fbb_descent = tm.font_bounding_box_descent();

    // Size the canvas so the whole ink box provably fits, with padding.
    let width = (abb_left + abb_right + 2.0 * PAD).ceil().max(1.0);
    let height = (abb_ascent + abb_descent + 2.0 * PAD).ceil().max(1.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas dimensions are small (well under u32::MAX), computed from finite emoji metrics"
    )]
    let canvas_width = width as u32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "canvas dimensions are small (well under u32::MAX), computed from finite emoji metrics"
    )]
    let canvas_height = height as u32;
    source_canvas.set_width(canvas_width);
    source_canvas.set_height(canvas_height);

    // Resizing the canvas resets its 2D context state, so re-apply font/baseline.
    source_ctx.set_font(&font);
    source_ctx.set_text_baseline("alphabetic");

    let origin_x = PAD + abb_left;
    let origin_y = PAD + abb_ascent;
    source_ctx.fill_text(EMOJI, origin_x, origin_y)?;

    let image_data: ImageData =
        source_ctx.get_image_data(0.0, 0.0, f64::from(canvas_width), f64::from(canvas_height))?;
    // Copy the pixel bytes into a Rust-owned buffer: this `Vec<u8>` living in Rust
    // memory *is* the proof that "Rust has the pixels".
    let pixels: Vec<u8> = image_data.data().0;

    let analysis = analyze_pixels(&pixels, canvas_width, canvas_height);

    let ink_box_consistent = match analysis.pixel_ink_box {
        Some((x0, y0, x1, y1)) => {
            // Cross-check the pixel-derived ink box against the TextMetrics box,
            // allowing generous slack for anti-aliasing/hinting differences.
            const SLACK: f64 = 8.0;
            let metrics_x0 = origin_x - abb_left;
            let metrics_x1 = origin_x + abb_right;
            let metrics_y0 = origin_y - abb_ascent;
            let metrics_y1 = origin_y + abb_descent;
            f64::from(x0) >= metrics_x0 - SLACK
                && f64::from(x1) <= metrics_x1 + SLACK
                && f64::from(y0) >= metrics_y0 - SLACK
                && f64::from(y1) <= metrics_y1 + SLACK
        }
        None => false,
    };

    let mut fail_reasons = Vec::new();
    if advance_width <= 1.0 {
        fail_reasons.push("advance width is near-zero".to_owned());
    }
    if analysis.colored_pixel_count == 0 {
        fail_reasons.push("no colored pixels found (fallback/tofu?)".to_owned());
    }
    if analysis.non_transparent_pixel_count == 0 {
        fail_reasons.push("no ink drawn at all".to_owned());
    }
    if analysis.border_clipped {
        fail_reasons.push("ink reached the canvas border (possible clipping)".to_owned());
    }
    if analysis.pixel_ink_box.is_none() {
        fail_reasons.push("could not determine a pixel ink box".to_owned());
    }
    if !ink_box_consistent {
        fail_reasons.push("pixel ink box inconsistent with TextMetrics box".to_owned());
    }

    let pass = fail_reasons.is_empty();

    let report = Report {
        advance_width,
        actual_bounding_box_left: abb_left,
        actual_bounding_box_right: abb_right,
        actual_bounding_box_ascent: abb_ascent,
        actual_bounding_box_descent: abb_descent,
        font_bounding_box_ascent: fbb_ascent,
        font_bounding_box_descent: fbb_descent,
        canvas_width,
        canvas_height,
        colored_pixel_count: analysis.colored_pixel_count,
        non_transparent_pixel_count: analysis.non_transparent_pixel_count,
        alpha_coverage_percent: analysis.alpha_coverage_percent,
        border_clipped: analysis.border_clipped,
        pixel_ink_box: analysis.pixel_ink_box,
        ink_box_consistent_with_metrics: ink_box_consistent,
        pass,
        fail_reasons,
    };

    // Round-trip proof: draw a magenta frame around the detected ink box directly
    // into the Rust-owned buffer, then reconstruct a fresh `ImageData` from it and
    // `putImageData` onto a second, independent canvas.
    let mut rust_pixels = pixels;
    if let Some(ink_box) = report.pixel_ink_box {
        draw_magenta_frame(&mut rust_pixels, canvas_width, canvas_height, ink_box);
    }

    let roundtrip_canvas = get_canvas(&document, "roundtrip")?;
    roundtrip_canvas.set_width(canvas_width);
    roundtrip_canvas.set_height(canvas_height);
    let roundtrip_ctx = get_2d_context(&roundtrip_canvas)?;
    let new_image_data = ImageData::new_with_u8_clamped_array_and_sh(
        Clamped(&rust_pixels),
        canvas_width,
        canvas_height,
    )?;
    roundtrip_ctx.put_image_data(&new_image_data, 0.0, 0.0)?;

    render_report(&document, &report)?;

    set_status(
        &document,
        if report.pass {
            "done — PASS"
        } else {
            "done — FAIL"
        },
    )?;

    Ok(())
}

/// Results of the pure-Rust analysis of the captured RGBA buffer.
struct PixelAnalysis {
    colored_pixel_count: u32,
    non_transparent_pixel_count: u32,
    alpha_coverage_percent: f64,
    border_clipped: bool,
    /// Inclusive-exclusive bounding box `(x0, y0, x1, y1)` of non-transparent pixels.
    pixel_ink_box: Option<(u32, u32, u32, u32)>,
}

/// Analyze an RGBA8 buffer of size `width x height` entirely in Rust: colored-pixel
/// count (proof of real color-emoji rendering, not a monochrome fallback), alpha
/// coverage, a border-pixel clip scan, and the pixel-derived ink bounding box.
fn analyze_pixels(pixels: &[u8], width: u32, height: u32) -> PixelAnalysis {
    let mut colored_pixel_count: u32 = 0;
    let mut non_transparent_pixel_count: u32 = 0;
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0_u32;
    let mut max_y = 0_u32;

    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let r = pixels[idx];
            let g = pixels[idx + 1];
            let b = pixels[idx + 2];
            let a = pixels[idx + 3];
            if a > 0 {
                non_transparent_pixel_count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);

                let max_channel = r.max(g).max(b);
                let min_channel = r.min(g).min(b);
                if max_channel - min_channel > COLOR_CHANNEL_DIFF_THRESHOLD {
                    colored_pixel_count += 1;
                }
            }
        }
    }

    let border_clipped = (0..width).any(|x| {
        pixel_alpha(pixels, width, x, 0) > 0 || pixel_alpha(pixels, width, x, height - 1) > 0
    }) || (0..height).any(|y| {
        pixel_alpha(pixels, width, 0, y) > 0 || pixel_alpha(pixels, width, width - 1, y) > 0
    });

    let total_pixels = f64::from(width) * f64::from(height);
    let alpha_coverage_percent = if total_pixels > 0.0 {
        100.0 * f64::from(non_transparent_pixel_count) / total_pixels
    } else {
        0.0
    };

    let pixel_ink_box = if non_transparent_pixel_count > 0 {
        Some((min_x, min_y, max_x + 1, max_y + 1))
    } else {
        None
    };

    PixelAnalysis {
        colored_pixel_count,
        non_transparent_pixel_count,
        alpha_coverage_percent,
        border_clipped,
        pixel_ink_box,
    }
}

fn pixel_alpha(pixels: &[u8], width: u32, x: u32, y: u32) -> u8 {
    let idx = ((y * width + x) * 4) as usize;
    pixels[idx + 3]
}

/// Draw a 1px-thick magenta frame directly into an RGBA8 buffer, around
/// `(x0, y0, x1, y1)` (exclusive on x1/y1). This is a purely Rust-side mutation of
/// the pixel bytes, so its presence in Canvas B proves Rust touched the buffer.
fn draw_magenta_frame(pixels: &mut [u8], width: u32, height: u32, (x0, y0, x1, y1): (u32, u32, u32, u32)) {
    const MAGENTA: [u8; 4] = [255, 0, 255, 255];
    let mut set_pixel = |x: u32, y: u32| {
        if x < width && y < height {
            let idx = ((y * width + x) * 4) as usize;
            pixels[idx..idx + 4].copy_from_slice(&MAGENTA);
        }
    };
    let x1 = x1.saturating_sub(1).max(x0);
    let y1 = y1.saturating_sub(1).max(y0);
    for x in x0..=x1 {
        set_pixel(x, y0);
        set_pixel(x, y1);
    }
    for y in y0..=y1 {
        set_pixel(x0, y);
        set_pixel(x1, y);
    }
}

fn render_report(document: &Document, report: &Report) -> Result<(), JsValue> {
    let metrics_el = document.get_element_by_id("metrics").ok_or("no #metrics")?;
    metrics_el.set_text_content(Some(&report.to_display_string()));

    let data_el = document
        .get_element_by_id("data-phase-one")
        .ok_or("no #data-phase-one")?;
    data_el.set_text_content(Some(&report.to_json()));

    Ok(())
}

fn get_canvas(document: &Document, id: &str) -> Result<HtmlCanvasElement, JsValue> {
    Ok(document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("no #{id}")))?
        .dyn_into::<HtmlCanvasElement>()?)
}

fn get_2d_context(canvas: &HtmlCanvasElement) -> Result<CanvasRenderingContext2d, JsValue> {
    Ok(canvas
        .get_context("2d")?
        .ok_or("no 2d context")?
        .dyn_into::<CanvasRenderingContext2d>()?)
}

fn set_status(document: &Document, message: &str) -> Result<(), JsValue> {
    if let Some(status) = document.get_element_by_id("status") {
        status.set_text_content(Some(message));
    }
    Ok(())
}

/// Yield to other tasks in the browser, allowing a paint to happen before we
/// continue. Adapted from
/// <https://github.com/wasm-bindgen/wasm-bindgen/discussions/3476#discussion-5283084>
async fn yield_now(window: &web_sys::Window) {
    let mut cb = |resolve: js_sys::Function, _reject: js_sys::Function| {
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .expect("Failed to call set_timeout");
    };
    let p = js_sys::Promise::new(&mut cb);
    let _ = JsFuture::from(p).await;
}
