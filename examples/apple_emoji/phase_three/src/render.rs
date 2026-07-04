// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! `vello_cpu` rendering pipeline (copied/adapted from `examples/vello_cpu_render`) plus
//! the emoji pixel blit into each `PositionedInlineBox`.

use std::sync::Arc;

use glifo::renderers::vello_renderer::replay_atlas_commands;
use glifo::{AtlasConfig, CpuGlyphCaches, GlyphCache, GlyphCacheConfig, GlyphRunBuilder, ImageCache};
use parley::{Layout, PositionedInlineBox, PositionedLayoutItem};
use peniko::Color;
use vello_cpu::{
    Pixmap, RenderContext,
    kurbo::{Affine, Rect, Vec2},
};

use crate::capture::EmojiCapture;
use crate::layout::ColorBrush;

/// Create the renderer and glyph caches (once per app).
pub(crate) fn prepare_rendering(width: u16, height: u16) -> (RenderContext, RenderContext, CpuGlyphCaches, ImageCache) {
    let atlas_size = (256, 256);
    let renderer = RenderContext::new(width, height);
    let image_cache = ImageCache::new_with_config(AtlasConfig {
        initial_atlas_count: 1,
        max_atlases: 1,
        atlas_size: (u32::from(atlas_size.0), u32::from(atlas_size.1)),
        auto_grow: false,
        ..Default::default()
    });
    let glyph_renderer = RenderContext::new(atlas_size.0, atlas_size.1);
    let glyph_caches = CpuGlyphCaches::with_config(
        256,
        256,
        GlyphCacheConfig {
            max_entry_age: 2,
            eviction_frequency: 2,
            max_cached_font_size: 128.0,
        },
    );
    (renderer, glyph_renderer, glyph_caches, image_cache)
}

fn reset_renderer(
    renderer: &mut RenderContext,
    glyph_renderer: &mut RenderContext,
    width: u16,
    height: u16,
    padding: u32,
    background_color: Color,
) {
    renderer.reset();
    glyph_renderer.reset();
    renderer.set_paint(background_color);
    renderer.fill_rect(&Rect::new(0.0, 0.0, f64::from(width), f64::from(height)));
    renderer.set_transform(Affine::translate(Vec2::new(
        f64::from(padding),
        f64::from(padding),
    )));
}

/// Render `layout` with `vello_cpu`, then blit captured emoji pixels into each inline
/// box. `captures[i]` must correspond to the inline box with `id == i as u64`.
#[expect(clippy::too_many_arguments, reason = "render orchestration requires many dependencies")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "inline box ids are small indices into `captures`, well under usize::MAX"
)]
pub(crate) fn render_layout(
    layout: &Layout<ColorBrush>,
    width: u16,
    height: u16,
    padding: u32,
    hint: bool,
    renderer: &mut RenderContext,
    glyph_renderer: &mut RenderContext,
    glyph_caches: &mut CpuGlyphCaches,
    image_cache: &mut ImageCache,
    captures: &[EmojiCapture],
    h_offsets: &[f32],
    v_shift_px: f32,
) -> Pixmap {
    reset_renderer(renderer, glyph_renderer, width, height, padding, Color::WHITE);

    let mut inline_boxes: Vec<PositionedInlineBox> = Vec::new();

    for line in layout.lines() {
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => {
                    let run = glyph_run.run();
                    let mut run_renderer =
                        GlyphRunBuilder::new(run.font().clone(), *renderer.transform())
                            .font_size(run.font_size())
                            .hint(hint)
                            .normalized_coords(run.normalized_coords())
                            .atlas_cache(true)
                            .build(
                                glyph_run.positioned_glyphs().map(|glyph| glifo::Glyph {
                                    id: glyph.id,
                                    x: glyph.x,
                                    y: glyph.y,
                                }),
                                glyph_caches,
                                image_cache,
                            );
                    renderer.set_paint(glyph_run.style().brush.color);
                    run_renderer.fill_glyphs(renderer);
                }
                PositionedLayoutItem::InlineBox(inline_box) => {
                    inline_boxes.push(inline_box);
                }
            }
        }
    }

    let mut pixmap = render(renderer, glyph_caches, image_cache, width, height, glyph_renderer);

    for ib in &inline_boxes {
        if let (Some(cap), Some(&h_offset)) = (captures.get(ib.id as usize), h_offsets.get(ib.id as usize)) {
            blit_emoji(&mut pixmap, cap, ib, padding, h_offset, v_shift_px);
        }
        // Comparison box (identical dimensions/definition to the HTML overlay drawn on
        // rows 1/3, see index.html): anchored at this row's own pen-left (`ib.x`) and
        // alphabetic baseline (`ib.y + ib.height`), letting ink size/placement be
        // compared visually across all three rows.
        let dev_left_x = f64::from(padding) + f64::from(ib.x);
        let dev_baseline_y = f64::from(padding) + f64::from(ib.y) + f64::from(ib.height);
        draw_comparison_box(&mut pixmap, dev_left_x, dev_baseline_y);
    }

    pixmap
}

/// Noto Color Emoji advance width at `FONT_SIZE` (`main.rs`'s `48.0`), in px:
/// `1275 / 1024 * 48`. Kept as a literal (not read from the generated advance table) so
/// the comparison box has one fixed size regardless of which emoji it's drawn around, per
/// the user's ruling that the box must be identical in every row/column.
const BOX_WIDTH: f64 = 1275.0 / 1024.0 * 48.0;
/// Noto Color Emoji ascent above the baseline at `FONT_SIZE`: `950 / 1024 * 48`.
const BOX_ASCENT: f64 = 950.0 / 1024.0 * 48.0;
/// Noto Color Emoji descent below the baseline at `FONT_SIZE`: `250 / 1024 * 48`.
const BOX_DESCENT: f64 = 250.0 / 1024.0 * 48.0;
/// Comparison-box outline color: magenta at 60% alpha, matching the HTML overlay's
/// `rgba(255, 0, 255, 0.6)`.
const BOX_COLOR: (f64, f64, f64, f64) = (255.0, 0.0, 255.0, 0.6);

/// Draw a 1px-outline comparison box (no fill) anchored at `(left_x, baseline_y)`
/// (device px, padding already applied): `[left_x, left_x + BOX_WIDTH] x
/// [baseline_y - BOX_ASCENT, baseline_y + BOX_DESCENT]`. `pixmap` is assumed opaque white
/// so straight-over blending needs no un-premultiply of the destination.
#[expect(
    clippy::cast_possible_truncation,
    reason = "pixmap dimensions are small (well under i64::MAX)"
)]
fn draw_comparison_box(pixmap: &mut Pixmap, left_x: f64, baseline_y: f64) {
    let x0 = left_x.round() as i64;
    let x1 = (left_x + BOX_WIDTH).round() as i64;
    let y0 = (baseline_y - BOX_ASCENT).round() as i64;
    let y1 = (baseline_y + BOX_DESCENT).round() as i64;

    let pw = pixmap.width() as i64;
    let ph = pixmap.height() as i64;
    let dst = pixmap.data_as_u8_slice_mut();

    let mut blend = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= pw || y >= ph {
            return;
        }
        let idx = ((y * pw + x) * 4) as usize;
        let (r, g, b, a) = BOX_COLOR;
        let inv = 1.0 - a;
        dst[idx] = (r * a + f64::from(dst[idx]) * inv).round().clamp(0.0, 255.0) as u8;
        dst[idx + 1] = (g * a + f64::from(dst[idx + 1]) * inv).round().clamp(0.0, 255.0) as u8;
        dst[idx + 2] = (b * a + f64::from(dst[idx + 2]) * inv).round().clamp(0.0, 255.0) as u8;
        dst[idx + 3] = 255;
    };

    for x in x0..=x1 {
        blend(x, y0);
        blend(x, y1);
    }
    for y in y0..=y1 {
        blend(x0, y);
        blend(x1, y);
    }
}

/// Rasterize to pixmap, maintain caches (copied from `vello_cpu_render`).
fn render(
    renderer: &mut RenderContext,
    glyph_caches: &mut CpuGlyphCaches,
    image_cache: &mut ImageCache,
    width: u16,
    height: u16,
    glyph_renderer: &mut RenderContext,
) -> Pixmap {
    glyph_caches
        .glyph_atlas
        .replay_pending_atlas_commands_with_pixmaps(|recorder, pixmaps| {
            glyph_renderer.reset();
            replay_atlas_commands(&mut recorder.commands, glyph_renderer);
            glyph_renderer.flush();
            if let Some(atlas_pixmap) = pixmaps
                .get_mut(recorder.page_index as usize)
                .and_then(Arc::get_mut)
            {
                glyph_renderer.composite_to_pixmap_at_offset(atlas_pixmap, 0, 0);
            }
        });

    let uploads: Vec<_> = glyph_caches.glyph_atlas.drain_pending_uploads().collect();
    for upload in uploads {
        let page_index = upload.atlas_slot.page_index as usize;
        let Some(atlas_pixmap) = glyph_caches.glyph_atlas.page_pixmap_mut(page_index) else {
            continue;
        };
        copy_pixmap_to_atlas(
            &upload.pixmap,
            atlas_pixmap,
            upload.atlas_slot.x,
            upload.atlas_slot.y,
            upload.atlas_slot.width,
            upload.atlas_slot.height,
        );
    }

    let page_count = glyph_caches.glyph_atlas.page_count();
    for page_index in 0..page_count {
        if let Some(page_pixmap) = glyph_caches.glyph_atlas.page_pixmap(page_index) {
            renderer.register_image(page_pixmap.clone());
        }
    }

    let mut pixmap = Pixmap::new(width, height);
    renderer.render_to_pixmap(&mut pixmap);
    renderer.clear_images();
    glyph_caches.maintain(image_cache);

    let clear_rects: Vec<_> = glyph_caches.glyph_atlas.drain_pending_clear_rects().collect();
    for rect in clear_rects {
        if let Some(atlas_pixmap) = glyph_caches
            .glyph_atlas
            .page_pixmap_mut(rect.page_index as usize)
        {
            clear_pixmap_region(atlas_pixmap, &rect);
        }
    }
    glyph_caches.glyph_atlas.clear_stats();

    pixmap
}

fn clear_pixmap_region(dst: &mut Pixmap, rect: &glifo::PendingClearRect) {
    let dst_stride = dst.width() as usize;
    let dst_data = dst.data_as_u8_slice_mut();
    let clear_width = rect.width as usize;
    let clear_height = rect.height as usize;
    for y in 0..clear_height {
        let row_start = ((rect.y as usize + y) * dst_stride + rect.x as usize) * 4;
        let row_end = row_start + clear_width * 4;
        dst_data[row_start..row_end].fill(0);
    }
}

fn copy_pixmap_to_atlas(src: &Pixmap, dst: &mut Pixmap, dst_x: u16, dst_y: u16, width: u16, height: u16) {
    let copy_width = width as usize;
    let copy_height = height as usize;
    let src_stride = src.width() as usize;
    let dst_stride = dst.width() as usize;
    let src_data = src.data_as_u8_slice();
    let dst_data = dst.data_as_u8_slice_mut();
    for y in 0..copy_height {
        let src_row_start = y * src_stride * 4;
        let src_row_end = src_row_start + copy_width * 4;
        let dst_row_start = ((dst_y as usize + y) * dst_stride + dst_x as usize) * 4;
        let dst_row_end = dst_row_start + copy_width * 4;
        dst_data[dst_row_start..dst_row_end].copy_from_slice(&src_data[src_row_start..src_row_end]);
    }
}

/// Blit captured emoji pixels (straight RGBA) into `pixmap` (premultiplied RGBA),
/// baseline-aligned and horizontally centered within the inline box via `h_offset`
/// (user ruling: the Apple emoji is captured at the fixed global scale
/// `FONT_SIZE * APPLE_TO_NOTO_EM_SCALE`, the same for every emoji, so this is always a
/// crisp 1:1 copy; `h_offset` is computed by the caller from the capture's own
/// pixel-measured ink bounding box, `capture::pixel_ink_bbox`, so that the ink's
/// horizontal center lands at the wider Noto-sized inline box's horizontal center).
/// `pixmap` is assumed opaque white so no un/premultiply round-trip is needed for the
/// destination read — only the source needs premultiplying before src-over.
///
/// `v_shift_px` (user ruling, option A): a global downward shift applied to the blit's
/// baseline only (not to the comparison box, which stays anchored to the shared Parley
/// baseline) — see `APPLE_TO_NOTO_BASELINE_SHIFT_EM` in `main.rs`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "pixmap/capture dimensions are small (well under i64/u8::MAX)"
)]
fn blit_emoji(
    pixmap: &mut Pixmap,
    cap: &EmojiCapture,
    ib: &PositionedInlineBox,
    padding: u32,
    h_offset: f32,
    v_shift_px: f32,
) {
    let dev_left_x = f64::from(padding) + f64::from(ib.x) + f64::from(h_offset);
    // ib.y == line_baseline - ib.height (Parley's inline-box vertical semantics), so
    // the baseline is exactly ib.y + ib.height. `v_shift_px` shifts only this blit's
    // baseline (user ruling, option A) — not the comparison box, which is drawn
    // separately using the unshifted `ib.y + ib.height`.
    let dev_baseline_y = f64::from(padding) + f64::from(ib.y) + f64::from(ib.height) + f64::from(v_shift_px);

    let pw = pixmap.width() as i64;
    let ph = pixmap.height() as i64;
    let dst = pixmap.data_as_u8_slice_mut();

    // Destination bounding box: forward-map the source capture's corners
    // (dx = dev_left_x + (sx - origin_x)) to find the dest pixel range to visit.
    let dx_min = (dev_left_x - cap.origin_x).floor() as i64;
    let dx_max = (dev_left_x + (f64::from(cap.canvas_width) - cap.origin_x)).ceil() as i64;
    let dy_min = (dev_baseline_y - cap.origin_y).floor() as i64;
    let dy_max = (dev_baseline_y + (f64::from(cap.canvas_height) - cap.origin_y)).ceil() as i64;

    for dy in dy_min.max(0)..dy_max.min(ph) {
        for dx in dx_min.max(0)..dx_max.min(pw) {
            // Inverse-map to the source capture (1:1, no resampling).
            let sx = (cap.origin_x + (dx as f64 + 0.5 - dev_left_x)).floor();
            let sy = (cap.origin_y + (dy as f64 + 0.5 - dev_baseline_y)).floor();
            if sx < 0.0 || sy < 0.0 {
                continue;
            }
            let (sx, sy) = (sx as u32, sy as u32);
            if sx >= cap.canvas_width || sy >= cap.canvas_height {
                continue;
            }
            let idx = ((sy * cap.canvas_width + sx) * 4) as usize;
            let a = cap.pixels[idx + 3];
            if a == 0 {
                continue;
            }
            let r = cap.pixels[idx];
            let g = cap.pixels[idx + 1];
            let b = cap.pixels[idx + 2];
            let af = f64::from(a) / 255.0;
            // Premultiply source.
            let sr = f64::from(r) * af;
            let sg = f64::from(g) * af;
            let sb = f64::from(b) * af;

            let dst_idx = ((dy * pw + dx) * 4) as usize;
            let dr = f64::from(dst[dst_idx]);
            let dg = f64::from(dst[dst_idx + 1]);
            let db = f64::from(dst[dst_idx + 2]);
            let da = f64::from(dst[dst_idx + 3]);
            let inv = 1.0 - af;

            dst[dst_idx] = (sr + dr * inv).round().clamp(0.0, 255.0) as u8;
            dst[dst_idx + 1] = (sg + dg * inv).round().clamp(0.0, 255.0) as u8;
            dst[dst_idx + 2] = (sb + db * inv).round().clamp(0.0, 255.0) as u8;
            dst[dst_idx + 3] = (f64::from(a) + da * inv).round().clamp(0.0, 255.0) as u8;
        }
    }
}
