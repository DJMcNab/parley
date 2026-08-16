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
        Affine::scale(f64::from(SCALE))
            * Affine::translate(Vec2::new(PADDING.into(), PADDING.into())),
    );
    context.set_paint(Color::BLACK);

    // One fragment is one `GlyphRunBuilder`: a fragment carries a single style by
    // construction, which is exactly what the builder is built per, and drawing them
    // separately means the image is laid out from the same origins the diff table
    // reports rather than from re-summed absolute positions.
    for fragment in &output.fragments {
        let style = output
            .styles
            .get(usize::from(fragment.style))
            .expect("a fragment's style index must be in range for its own output");
        let Some(font) = fonts.get(&style.postscript_name) else {
            continue;
        };
        GlyphRunBuilder::new(font.clone(), *context.transform())
            .font_size(style.font_size)
            .hint(false)
            .build(
                fragment.glyphs.iter().map(|glyph| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "rasterisation is f32 throughout; the f64 origin exists so \
                                  the comparison does not re-round it, not for drawing"
                    )]
                    glifo::Glyph {
                        id: glyph.id,
                        x: (fragment.origin_x + f64::from(glyph.x)) as f32,
                        y: (fragment.origin_y + f64::from(glyph.y)) as f32,
                    }
                }),
                &mut caches,
                &mut image_cache,
            )
            .fill_glyphs(&mut context);
    }

    let mut pixmap = Pixmap::new(width, height);
    context.flush();
    context.render_to_pixmap(&mut pixmap);
    pixmap
        .into_png()
        .expect("encoding an in-memory pixmap as PNG cannot fail")
}
