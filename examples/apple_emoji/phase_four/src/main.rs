// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Phase Four: draw text with Parley + `vello_cpu` where emoji stay **in the text** and
//! are shaped as real glyphs from a tiny fake-Noto-metrics font (Plan A, zero Parley
//! changes), then at render time each fake-Noto `GlyphRun` is intercepted by font
//! identity (`Blob::id()`) and its clusters replaced by baseline-correct Apple system
//! emoji pixels, captured at a fixed global scale of the plain-text `FONT_SIZE` (user
//! ruling, superseding the earlier per-emoji ink-matched-scaling ruling — see
//! `APPLE_TO_NOTO_EM_SCALE`), horizontally centered within the Noto-advance cluster width
//! by the capture's own pixel-measured ink bounding box.
//!
//! See `plans/phase_four_plan.md` for the full design.

mod capture;
mod layout;
mod render;

use parley::{FontContext, LayoutContext};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Document, HtmlCanvasElement, ImageData};

use capture::{capture_emoji, pixel_ink_bbox, EmojiCapture};
use layout::{build_layout, is_emoji_char, register_fonts};
use render::blit_emoji;

/// Example string: single-`char` emoji only (documented simplification carried from
/// earlier phases — ZWJ/multi-scalar emoji sequences are out of scope, see plan §3.3/R3).
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

    set_status(&document, "Building layout…")?;

    // 1. Register Arimo + the fake-Noto font; capture the fake font blob's id for the
    //    render-time font-identity check (Plan A, §5, R5).
    let mut font_cx = FontContext::new();
    let mut layout_cx = LayoutContext::new();
    let (arimo_family, fake_font_blob_id) = register_fonts(&mut font_cx);

    // 2. Build the layout with emoji left in the text, styled with an explicit
    //    `FakeNotoEmoji` family span per emoji char (Q5).
    let layout = build_layout(
        &mut font_cx,
        &mut layout_cx,
        EXAMPLE_TEXT,
        &arimo_family,
        FONT_SIZE,
        None,
    );

    let width = layout.width().ceil() as u16 + (PADDING * 2) as u16;
    let height = layout.height().ceil() as u16 + (PADDING * 2) as u16;
    // Shared Parley baseline for the (single) line, in canvas-pixel space (padding
    // included) — exported so validate.py can directly assert each emoji's blitted
    // baseline (below) equals this value (team-lead ruling: baseline-on-baseline is now
    // the vertical-placement acceptance criterion, superseding the old ink-band check).
    let line_baseline = layout
        .lines()
        .next()
        .map(|line| line.metrics().baseline)
        .unwrap_or(0.0)
        + f32::from(PADDING as u16);

    set_status(&document, "Rendering…")?;

    // 3. Render with vello_cpu. Fake-Noto glyph runs are skipped in the glyph pass and
    //    their emoji clusters returned as `EmojiBlit`s (Plan A, §5).
    let (mut renderer, mut glyph_renderer, mut glyph_caches, mut image_cache) =
        render::prepare_rendering(width, height);
    let (mut pixmap, emoji_blits) = render::render_layout(
        &layout,
        width,
        height,
        PADDING,
        true,
        &mut renderer,
        &mut glyph_renderer,
        &mut glyph_caches,
        &mut image_cache,
        fake_font_blob_id,
    );

    set_status(&document, "Capturing + blitting emoji…")?;

    // 4. For each recovered emoji cluster, capture the Apple system emoji at the fixed
    //    global scale `FONT_SIZE * APPLE_TO_NOTO_EM_SCALE` (user ruling; see the constant's
    //    doc comment) and blit 1:1 (no pixel resampling). The capture's own alphabetic
    //    baseline is placed on the shared Parley baseline (earlier team-lead ruling,
    //    supersedes plan §6 — no ink-band fitting, no vertical margins, no vertical
    //    scaling). Horizontal placement centers the capture's own *pixel-measured* ink
    //    bounding box (`pixel_ink_bbox`, a per-pixel scan of the rasterized capture)
    //    within the Noto-advance cluster width — `TextMetrics.actualBoundingBox*` is not
    //    used for this because headless Chrome's implementation for Apple Color Emoji has
    //    been observed to fall back to a constant unrelated to the glyph's real ink (see
    //    `examples/apple_emoji/ink_survey/noto_analysis.txt`).
    let scratch_canvas = get_canvas(&document, "scratch")?;
    let capture_font_size = f64::from(FONT_SIZE) * f64::from(APPLE_TO_NOTO_EM_SCALE);
    // Global downward baseline shift applied at blit time (see the constant's doc
    // comment); computed once here in px at `FONT_SIZE`.
    let v_shift_px = APPLE_TO_NOTO_BASELINE_SHIFT_EM * FONT_SIZE;
    let mut per_emoji_json = Vec::new();
    let mut emoji_advance_sum = 0.0_f32;
    for blit in &emoji_blits {
        let ch = char::from_u32(blit.cp).ok_or("invalid emoji codepoint")?;
        let mut buf = [0_u8; 4];
        let s: &str = ch.encode_utf8(&mut buf);
        let cap: EmojiCapture = capture_emoji(&document, &scratch_canvas, s, capture_font_size)?;

        // Center the capture's pixel-measured ink horizontally within the Noto-advance
        // cluster width: shift so the ink's horizontal center lands at the advance box's
        // horizontal center. `h_offset = noto_advance/2 - (pixel_ink.center_x - origin_x)`
        // is algebraically the same thing as `(noto_advance - pixel_ink_width) / 2 -
        // (pixel_ink_left - origin_x)`.
        let ink = pixel_ink_bbox(&cap);
        let (pixel_ink_width, pixel_ink_height, h_offset) = match &ink {
            Some(ink) => {
                let h_offset = blit.noto_advance_px / 2.0
                    - (ink.center_x() - cap.origin_x) as f32;
                (ink.width(), ink.height(), h_offset)
            }
            // No non-transparent pixels found (shouldn't happen for a real emoji glyph);
            // fall back to centering the whole capture canvas instead of failing.
            None => (0.0, 0.0, (blit.noto_advance_px - cap.canvas_width as f32) / 2.0),
        };

        blit_emoji(&mut pixmap, &cap, blit, PADDING, h_offset, v_shift_px);
        // Comparison box (identical dimensions/definition to the HTML overlay drawn on
        // rows 1/3, see index.html): anchored at this emoji's own pen-left (`blit.x`,
        // i.e. the cluster's advance-box left edge, not the ink-centered blit position)
        // and the shared Parley baseline (`blit.baseline`).
        render::draw_comparison_box(
            &mut pixmap,
            f64::from(PADDING) + f64::from(blit.x),
            f64::from(PADDING) + f64::from(blit.baseline),
        );

        emoji_advance_sum += blit.noto_advance_px;
        // Report canvas-pixel-space coordinates (i.e. with `PADDING` already added, matching
        // where the emoji actually lands in the rendered/screenshotted canvas), so
        // validate.py can map directly via the canvas's on-screen bounding rect.
        let padded_x = blit.x + f32::from(PADDING as u16);
        let padded_line_baseline = blit.baseline + f32::from(PADDING as u16);
        // `baseline` is the emoji's *actual* blitted baseline (shared Parley line
        // baseline + the global downward shift) — this is what the emoji pixels are
        // really placed at, not the unshifted line baseline (see `line_baseline` at the
        // top level of this JSON for that).
        let padded_emoji_baseline = padded_line_baseline + v_shift_px;
        per_emoji_json.push(format!(
            "{{\"char\":{:?},\"noto_advance_px\":{},\"apple_advance_width\":{},\
             \"capture_font_size\":{},\"pixel_ink_width\":{},\"pixel_ink_height\":{},\
             \"h_offset\":{},\"x\":{},\"baseline\":{},\"v_shift_px\":{}}}",
            ch.to_string(),
            blit.noto_advance_px,
            cap.advance_width,
            capture_font_size,
            pixel_ink_width,
            pixel_ink_height,
            h_offset,
            padded_x,
            padded_emoji_baseline,
            v_shift_px,
        ));
    }

    // 5. Present via putImageData. The pixmap is rendered on an opaque white background,
    //    so premultiplied == straight and bytes can be copied directly.
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

    // 6. Sanity: confirm every codepoint we styled as an emoji actually got a fake-Noto
    //    glyph run (i.e. the font-identity interception found it) — surfaced as a hard
    //    failure if the count doesn't match (this is the R5/R4 check: font-identity
    //    survived registration and family selection worked).
    let expected_emoji = EXAMPLE_TEXT.chars().filter(|&c| is_emoji_char(c)).count();
    if emoji_blits.len() != expected_emoji {
        return Err(JsValue::from_str(&format!(
            "font-identity/family-selection mismatch: expected {expected_emoji} fake-Noto \
             emoji clusters, found {}",
            emoji_blits.len()
        )));
    }

    let json = format!(
        "{{\"font_size\":{},\"layout_width\":{},\"layout_height\":{},\"emoji_advance_sum\":{},\
         \"line_baseline\":{},\"apple_to_noto_em_scale\":{},\"apple_to_noto_baseline_shift_em\":{},\
         \"v_shift_px\":{},\"emoji\":[{}]}}",
        FONT_SIZE,
        layout.width(),
        layout.height(),
        emoji_advance_sum,
        line_baseline,
        APPLE_TO_NOTO_EM_SCALE,
        APPLE_TO_NOTO_BASELINE_SHIFT_EM,
        v_shift_px,
        per_emoji_json.join(",")
    );
    let data_el = document
        .get_element_by_id("data-phase-four")
        .ok_or("no #data-phase-four")?;
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
