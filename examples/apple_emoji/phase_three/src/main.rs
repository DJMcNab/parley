// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase Three: draw text with Parley + `vello_cpu` where each emoji is an inline box
//! sized to its **Noto Color Emoji** advance width (from a tiny generated table), filled
//! at blit time with Apple's real system emoji pixels (Phase-One/Two technique) captured
//! at a fixed global scale of the plain-text `FONT_SIZE` (user ruling, superseding the
//! earlier per-emoji ink-matched-scaling ruling — see `APPLE_TO_NOTO_EM_SCALE`) and
//! horizontally centered within the (wider) Noto advance box by the capture's own
//! pixel-measured ink bounding box. Presented beside a browser-native rendering of the
//! same string, plus an in-page real-Noto reference measurement, for validation.
//!
//! See `plans/phase_three_plan.md` for the full design.

mod capture;
mod layout;
mod noto_advances;
mod render;

use parley::{FontContext, LayoutContext, PositionedLayoutItem};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, HtmlCanvasElement, ImageData};

use capture::{capture_emoji, pixel_ink_bbox, EmojiCapture};
use layout::{build_layout, noto_advance_px, register_arimo, strip_emoji, BoxSpec};

/// Example string: single-`char` emoji only (documented Phase Two simplification —
/// ZWJ/multi-scalar emoji sequences are out of scope).
const EXAMPLE_TEXT: &str = "Hi \u{1F600} world \u{1F389}!";

const FONT_SIZE: f32 = 48.0;
const PADDING: u32 = 20;

/// Global Apple-system-emoji-to-Noto em scale (user ruling): every Apple emoji is
/// captured at `FONT_SIZE * APPLE_TO_NOTO_EM_SCALE`, regardless of which emoji it is.
///
/// A per-emoji survey (`examples/apple_emoji/ink_survey/noto_analysis.txt`, 1225 emoji
/// present in both Apple's and Noto's color emoji sets) found headless Chrome's
/// `TextMetrics` ink bounds for Apple Color Emoji fall back to a constant regardless of
/// glyph content, so a per-emoji ink-matched ratio (the earlier approach) was measuring
/// noise, not signal. A single global scale is both correct in spirit (it preserves
/// Apple's own intra-set size relationships instead of independently distorting each
/// glyph) and empirically tight (per-emoji real pixel-ink-width ratio Noto/Apple: median
/// 1.125, mean 1.136, CV 0.107 — i.e. most individual emoji land within ~11% of this
/// global value). `1.125` is that median ratio.
const APPLE_TO_NOTO_EM_SCALE: f32 = 1.125;

/// Global downward baseline shift (em-relative, user ruling option A): every Apple
/// emoji blit is shifted down by `APPLE_TO_NOTO_BASELINE_SHIFT_EM * FONT_SIZE` from the
/// shared Parley baseline, so the emoji sit in Noto's vertical band instead of on the
/// alphabetic baseline.
///
/// Derived from the same survey (`examples/apple_emoji/ink_survey/noto_analysis.txt`):
/// median Noto ink descent below baseline is `10/48` em, vs. the scaled median Apple
/// descent of `6 * 1.125 / 48` em; the difference is `≈0.068em`, rounded to `0.07`. This
/// is numerically equivalent to aligning the vertical centers of the two fonts' median
/// ink bands (Noto ascent 43 / descent 10 @48px vs. Apple ascent 40 / descent 5 @48px,
/// scaled by `APPLE_TO_NOTO_EM_SCALE`).
const APPLE_TO_NOTO_BASELINE_SHIFT_EM: f32 = 0.07;

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

    // 3. For each emoji: capture Apple's system emoji at the fixed global scale
    //    `FONT_SIZE * APPLE_TO_NOTO_EM_SCALE` (user ruling; see the constant's doc
    //    comment) — still browser-rasterized, still blitted 1:1 with no pixel
    //    resampling. The inline box stays sized to the Noto Color Emoji advance (from
    //    the embedded table, unchanged layout metric) and to the plain-`FONT_SIZE`
    //    `fontBoundingBoxAscent` (vertical placement is unchanged by the horizontal
    //    scale ruling, so this needs its own plain-size measurement); the Apple capture
    //    is centered inside that box at blit time via `h_offset`, computed from the
    //    capture's own pixel-measured ink bounding box (see render::blit_emoji) rather
    //    than `TextMetrics.actualBoundingBox*`, which is unreliable for system color
    //    emoji in headless Chrome (see `examples/apple_emoji/ink_survey/noto_analysis.txt`).
    let scratch_canvas = get_canvas(&document, "scratch")?;
    let capture_font_size = f64::from(FONT_SIZE) * f64::from(APPLE_TO_NOTO_EM_SCALE);
    let mut captures: Vec<EmojiCapture> = Vec::with_capacity(hits.len());
    let mut boxes: Vec<BoxSpec> = Vec::with_capacity(hits.len());
    let mut h_offsets: Vec<f32> = Vec::with_capacity(hits.len());
    let mut pixel_ink_widths: Vec<f64> = Vec::with_capacity(hits.len());
    let mut pixel_ink_heights: Vec<f64> = Vec::with_capacity(hits.len());
    for (i, hit) in hits.iter().enumerate() {
        let mut buf = [0_u8; 4];
        let s: &str = hit.ch.encode_utf8(&mut buf);

        // Plain-`FONT_SIZE` measurement, used only for the inline box's vertical extent
        // (fontBoundingBoxAscent) — vertical placement/layout metrics are unaffected by
        // the global horizontal scale ruling.
        let plain_metrics_cap = capture_emoji(&document, &scratch_canvas, s, f64::from(FONT_SIZE))?;
        let natural_fbb_ascent = plain_metrics_cap.fbb_ascent;

        let cap = capture_emoji(&document, &scratch_canvas, s, capture_font_size)?;

        let noto_px = noto_advance_px(hit.ch as u32, FONT_SIZE)
            .ok_or_else(|| JsValue::from_str(&format!("{:?} not covered by Noto table", hit.ch)))?;
        // Center the capture's pixel-measured ink horizontally within the Noto advance
        // box: shift so the ink's horizontal center lands at the box's horizontal
        // center. `h_offset = noto_px/2 - (pixel_ink.center_x - origin_x)` is
        // algebraically the same thing as `(noto_px - pixel_ink_width) / 2 -
        // (pixel_ink_left - origin_x)`.
        let ink = pixel_ink_bbox(&cap);
        let (pixel_ink_width, pixel_ink_height, h_offset) = match &ink {
            Some(ink) => {
                let h_offset = noto_px / 2.0 - (ink.center_x() - cap.origin_x) as f32;
                (ink.width(), ink.height(), h_offset)
            }
            // No non-transparent pixels found (shouldn't happen for a real emoji glyph);
            // fall back to centering the whole capture canvas instead of failing.
            None => (0.0, 0.0, (noto_px - cap.canvas_width as f32) / 2.0),
        };
        boxes.push(BoxSpec {
            id: i as u64,
            stripped_byte_index: hit.stripped_byte_index,
            width: noto_px,
            // Box bottom sits on the baseline (Parley semantics); fontBoundingBoxAscent
            // at plain FONT_SIZE (unchanged — vertical/layout metrics are unaffected by
            // the horizontal scale ruling) so the line grows to match native
            // emoji-line sizing.
            height: natural_fbb_ascent as f32,
        });
        h_offsets.push(h_offset);
        pixel_ink_widths.push(pixel_ink_width);
        pixel_ink_heights.push(pixel_ink_height);
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
    // Global downward baseline shift applied at blit time (see the constant's doc
    // comment); computed once here in px at `FONT_SIZE`.
    let v_shift_px = APPLE_TO_NOTO_BASELINE_SHIFT_EM * FONT_SIZE;
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
        &h_offsets,
        v_shift_px,
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

    // 7. Collect per-emoji metrics for #data-phase-three, matching PositionedInlineBox
    //    id -> capture. Includes the Noto table advance and the horizontal centering
    //    `h_offset` applied at blit time so validate.py can cross-check against a
    //    real-Noto in-page measurement.
    let mut per_emoji = Vec::new();
    let mut emoji_advance_sum = 0.0_f32;
    for line in layout.lines() {
        for item in line.items() {
            if let PositionedLayoutItem::InlineBox(ib) = item
                && let (Some(cap), Some(hit), Some(&h_offset)) = (
                    captures.get(ib.id as usize),
                    hits.get(ib.id as usize),
                    h_offsets.get(ib.id as usize),
                )
            {
                let pixel_ink_width = pixel_ink_widths.get(ib.id as usize).copied().unwrap_or(0.0);
                let pixel_ink_height = pixel_ink_heights.get(ib.id as usize).copied().unwrap_or(0.0);
                emoji_advance_sum += ib.width;
                per_emoji.push(format!(
                    "{{\"id\":{},\"char\":{:?},\"apple_advance_width\":{},\
                     \"noto_advance_px\":{},\"h_offset\":{},\"capture_font_size\":{},\
                     \"pixel_ink_width\":{},\"pixel_ink_height\":{},\
                     \"box_x\":{},\"box_y\":{},\"box_width\":{},\
                     \"box_height\":{},\"parley_baseline_y\":{},\"v_shift_px\":{},\
                     \"emoji_baseline_y\":{}}}",
                    ib.id,
                    hit.ch.to_string(),
                    cap.advance_width,
                    ib.width,
                    h_offset,
                    capture_font_size,
                    pixel_ink_width,
                    pixel_ink_height,
                    ib.x,
                    ib.y,
                    ib.width,
                    ib.height,
                    ib.y + ib.height,
                    v_shift_px,
                    ib.y + ib.height + v_shift_px,
                ));
            }
        }
    }
    let json = format!(
        "{{\"font_size\":{},\"layout_width\":{},\"layout_height\":{},\"emoji_advance_sum\":{},\
         \"apple_to_noto_em_scale\":{},\"apple_to_noto_baseline_shift_em\":{},\"v_shift_px\":{},\
         \"emoji\":[{}]}}",
        FONT_SIZE,
        layout.width(),
        layout.height(),
        emoji_advance_sum,
        APPLE_TO_NOTO_EM_SCALE,
        APPLE_TO_NOTO_BASELINE_SHIFT_EM,
        v_shift_px,
        per_emoji.join(",")
    );
    let data_el = document
        .get_element_by_id("data-phase-three")
        .ok_or("no #data-phase-three")?;
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
