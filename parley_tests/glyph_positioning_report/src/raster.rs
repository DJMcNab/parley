// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Rasterising a [`GlyphOutput`] to a transparent PNG.
//!
//! **Both sides go through this one function.** Chrome's side has no `Layout` to render
//! from — only `(id, x, y, size)` and a font — so rendering Parley's from its `Layout`
//! instead would mean two code paths, and Parley's picture would then be drawn from
//! `positioned_glyphs`' f32 running offset rather than from the f64-accumulated numbers
//! in the page's diff table. At the magnitudes this report exists to show, the image
//! could then contradict the table. Feeding both sides the same `GlyphOutput` means any
//! difference between the two images is a position difference and nothing else.
//!
//! Hinting is off: Chrome's captures come from a browser run with
//! `--font-render-hinting=none`, so hinting here would move glyphs the recording never
//! moved.

use std::collections::HashMap;
use std::sync::Arc;

use glifo::{AtlasConfig, CpuGlyphCaches, GlyphRunBuilder, ImageCache};
use parley_glyph_positioning_cases::{FONTS, GlyphOutput, postscript_name_from_bytes};
use peniko::{
    Blob, Color, FontData,
    kurbo::{Affine, Vec2},
};
use vello_cpu::{Pixmap, RenderContext};

use crate::analysis::{Geometry, PADDING, SCALE};

/// Resolves the PostScript names appearing in a [`GlyphOutput`]'s style table back to
/// font data.
#[derive(Debug)]
pub(crate) struct Fonts {
    by_postscript_name: HashMap<String, FontData>,
}

impl Fonts {
    /// Indexes every registered font by its PostScript name.
    pub(crate) fn new() -> Self {
        let by_postscript_name = FONTS
            .iter()
            .map(|font| {
                let name = postscript_name_from_bytes(font.bytes, 0)
                    .expect("a registered font must have a readable PostScript name");
                let data = FontData::new(Blob::new(Arc::new(font.bytes.to_vec())), 0);
                (name, data)
            })
            .collect();
        Self { by_postscript_name }
    }

    /// The font data for a style's PostScript name.
    fn get(&self, postscript_name: &str) -> Option<&FontData> {
        self.by_postscript_name.get(postscript_name)
    }
}

/// Rasterises `output` into a transparent PNG of `geometry`, at [`SCALE`].
///
/// The glyphs are painted black onto transparency, so the page can put the pastel
/// cluster boxes genuinely *behind* them rather than faking the layering with a blend
/// mode, and so the two sides can be tinted and stacked in CSS to form the overlay.
pub(crate) fn render(output: &GlyphOutput, geometry: Geometry, fonts: &Fonts) -> Vec<u8> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "case geometry is bounded by MAX_CASE_WIDTH_PX and a 30px font"
    )]
    let (width, height) = (
        (geometry.width * SCALE).ceil() as u16,
        (geometry.height * SCALE).ceil() as u16,
    );

    let mut context = RenderContext::new(width, height);
    let mut caches = CpuGlyphCaches::default();
    let mut image_cache = ImageCache::new_with_config(AtlasConfig::default());
    context.set_transform(
        Affine::scale(f64::from(SCALE)) * Affine::translate(Vec2::new(PADDING.into(), PADDING.into())),
    );
    context.set_paint(Color::BLACK);

    // Consecutive glyphs sharing a style become one run: a style change is a font or
    // size change, which is exactly what a `GlyphRunBuilder` is built per.
    let mut start = 0;
    while start < output.glyphs.len() {
        let style_index = output.glyphs[start].style;
        let end = output.glyphs[start..]
            .iter()
            .position(|glyph| glyph.style != style_index)
            .map_or(output.glyphs.len(), |offset| start + offset);

        let style = output
            .styles
            .get(usize::from(style_index))
            .expect("a glyph's style index must be in range for its own output");
        if let Some(font) = fonts.get(&style.postscript_name) {
            GlyphRunBuilder::new(font.clone(), *context.transform())
                .font_size(style.font_size)
                .hint(false)
                .build(
                    output.glyphs[start..end].iter().map(|glyph| glifo::Glyph {
                        id: glyph.id,
                        x: glyph.x,
                        y: glyph.y,
                    }),
                    &mut caches,
                    &mut image_cache,
                )
                .fill_glyphs(&mut context);
        }
        start = end;
    }

    let mut pixmap = Pixmap::new(width, height);
    context.flush();
    context.render_to_pixmap(&mut pixmap);
    pixmap
        .into_png()
        .expect("encoding an in-memory pixmap as PNG cannot fail")
}
