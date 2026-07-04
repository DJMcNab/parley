// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase Two: draw text with Parley + `vello_cpu` where each emoji is an inline box,
//! filled at blit time with Apple's real system emoji pixels captured via a 2D canvas
//! (Phase-One technique). Presented beside a browser-native rendering of the same
//! string (Arimo `@font-face` + system emoji fallback) for visual comparison.
//!
//! See `plans/phase_two_plan.md` for the full design.

mod capture;
mod layout;
mod render;

use parley::{FontContext, LayoutContext, PositionedLayoutItem};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, HtmlCanvasElement, ImageData};

use capture::{capture_emoji, EmojiCapture};
use layout::{build_layout, register_arimo, strip_emoji, BoxSpec};

/// Example string: single-`char` emoji only (documented Phase Two simplification —
/// ZWJ/multi-scalar emoji sequences are out of scope).
const EXAMPLE_TEXT: &str = "Hi \u{1F600} world \u{1F389}!";

const FONT_SIZE: f32 = 48.0;
const PADDING: u32 = 20;

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

#[expect(
    clippy::cast_possible_truncation,
    reason = "layout/canvas dimensions and emoji indices are small, well under the target integer types' ranges"
)]
async fn run() -> Result<(), JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    set_status(&document, "Waiting for fonts to be ready…")?;
    if let Ok(ready) = document.fonts().ready() {
        let _ = JsFuture::from(ready).await;
    }
    yield_now(&window).await;

    set_status(&document, "Capturing emoji…")?;

    // 1. Native reference: same full string, Arimo via @font-face, emoji via system
    //    fallback. It's plain HTML already in index.html; nothing to do here.

    // 2. Strip emoji from the example text, recording byte offsets in the stripped text.
    let (stripped_text, hits) = strip_emoji(EXAMPLE_TEXT);

    // 3. Capture each emoji's Apple-system advance + pixels at FONT_SIZE.
    let scratch_canvas = get_canvas(&document, "scratch")?;
    let mut captures: Vec<EmojiCapture> = Vec::with_capacity(hits.len());
    let mut boxes: Vec<BoxSpec> = Vec::with_capacity(hits.len());
    for (i, hit) in hits.iter().enumerate() {
        let mut buf = [0_u8; 4];
        let s: &str = hit.ch.encode_utf8(&mut buf);
        let cap = capture_emoji(&document, &scratch_canvas, s, f64::from(FONT_SIZE))?;
        boxes.push(BoxSpec {
            id: i as u64,
            stripped_byte_index: hit.stripped_byte_index,
            width: cap.advance_width as f32,
            // Box bottom sits on the baseline (Parley semantics); use
            // fontBoundingBoxAscent so the line grows to match native emoji-line sizing.
            height: cap.fbb_ascent as f32,
        });
        captures.push(cap);
    }

    set_status(&document, "Building layout…")?;

    // 4. Build the Parley layout.
    let mut font_cx = FontContext::new();
    let mut layout_cx = LayoutContext::new();
    let family_name = register_arimo(&mut font_cx);
    let layout = build_layout(
        &mut font_cx,
        &mut layout_cx,
        &stripped_text,
        &family_name,
        FONT_SIZE,
        &boxes,
        None,
    );

    let width = layout.width().ceil() as u16 + (PADDING * 2) as u16;
    let height = layout.height().ceil() as u16 + (PADDING * 2) as u16;

    set_status(&document, "Rendering…")?;

    // 5. Render with vello_cpu + blit emoji.
    let (mut renderer, mut glyph_renderer, mut glyph_caches, mut image_cache) =
        render::prepare_rendering(width, height);
    let pixmap = render::render_layout(
        &layout,
        width,
        height,
        PADDING,
        true,
        &mut renderer,
        &mut glyph_renderer,
        &mut glyph_caches,
        &mut image_cache,
        &captures,
    );

    // 6. Present via putImageData. The pixmap is rendered on an opaque white
    //    background, so premultiplied == straight and bytes can be copied directly.
    let parley_canvas = get_canvas(&document, "parley")?;
    parley_canvas.set_width(u32::from(width));
    parley_canvas.set_height(u32::from(height));
    let ctx = capture::get_2d_context(&parley_canvas)?;
    let rgba = pixmap.data_as_u8_slice().to_vec();
    let image_data = ImageData::new_with_u8_clamped_array_and_sh(
        Clamped(&rgba),
        u32::from(width),
        u32::from(height),
    )?;
    ctx.put_image_data(&image_data, 0.0, 0.0)?;

    // 7. Collect per-emoji metrics for #data-phase-two, matching PositionedInlineBox
    //    id -> capture.
    let mut per_emoji = Vec::new();
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::InlineBox(ib) = item
                && let (Some(cap), Some(hit)) =
                    (captures.get(ib.id as usize), hits.get(ib.id as usize))
            {
                per_emoji.push(format!(
                    "{{\"id\":{},\"char\":{:?},\"advance_width\":{},\"abb_ascent\":{},\
                     \"abb_descent\":{},\"box_x\":{},\"box_y\":{},\"box_width\":{},\
                     \"box_height\":{},\"parley_baseline_y\":{}}}",
                    ib.id,
                    hit.ch.to_string(),
                    cap.advance_width,
                    cap.abb_ascent,
                    cap.abb_descent,
                    ib.x,
                    ib.y,
                    ib.width,
                    ib.height,
                    ib.y + ib.height,
                ));
            }
        }
    }
    let json = format!(
        "{{\"font_size\":{},\"layout_width\":{},\"layout_height\":{},\"emoji\":[{}]}}",
        FONT_SIZE,
        layout.width(),
        layout.height(),
        per_emoji.join(",")
    );
    let data_el = document
        .get_element_by_id("data-phase-two")
        .ok_or("no #data-phase-two")?;
    data_el.set_text_content(Some(&json));

    set_status(&document, "done")?;

    Ok(())
}

fn get_canvas(document: &Document, id: &str) -> Result<HtmlCanvasElement, JsValue> {
    Ok(document
        .get_element_by_id(id)
        .ok_or_else(|| JsValue::from_str(&format!("no #{id}")))?
        .dyn_into::<HtmlCanvasElement>()?)
}

fn set_status(document: &Document, message: &str) -> Result<(), JsValue> {
    if let Some(status) = document.get_element_by_id("status") {
        status.set_text_content(Some(message));
    }
    Ok(())
}

async fn yield_now(window: &web_sys::Window) {
    let mut cb = |resolve: js_sys::Function, _reject: js_sys::Function| {
        window
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 0)
            .expect("Failed to call set_timeout");
    };
    let p = js_sys::Promise::new(&mut cb);
    let _ = JsFuture::from(p).await;
}
