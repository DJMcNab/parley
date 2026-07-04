// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Build the Parley layout: Arimo text with emoji left **in the text** and shaped from
//! the fake-Noto font via an explicit `FontFamily::named("FakeNotoEmoji")` style span
//! (Phase Four, Q5 ruling) — no inline boxes.

use std::sync::Arc;

use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, Layout, LayoutContext, LineHeight,
    RangedBuilder, StyleProperty,
};
use peniko::Color;

/// Minimal brush type, matching `examples/common`'s `ColorBrush`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorBrush {
    pub color: Color,
}

impl Default for ColorBrush {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
        }
    }
}

const ARIMO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf"
));

const FAKE_NOTO: &[u8] = include_bytes!("fake_noto.ttf");

pub(crate) const FAKE_NOTO_FAMILY: &str = "FakeNotoEmoji";

/// Very small "is this an emoji" check, sufficient for the single-`char` emoji used in
/// `EXAMPLE_TEXT` — documented simplification carried from `phase_two/three` (ZWJ /
/// multi-scalar emoji are out of scope, see plan §3.3/R3).
pub(crate) fn is_emoji_char(c: char) -> bool {
    matches!(
        c as u32,
        0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x2190..=0x21FF | 0x2B00..=0x2BFF
    )
}

/// Register Arimo and the fake-Noto font into `font_cx`, returning
/// `(arimo_family_name, fake_font_blob_id)`. The blob id is used by the renderer for the
/// font-identity check (Plan A, §5/R5).
pub(crate) fn register_fonts(font_cx: &mut FontContext) -> (String, u64) {
    let arimo_families = font_cx
        .collection
        .register_fonts(Blob::new(Arc::new(ARIMO.to_vec())), None);
    let arimo_name = arimo_families
        .first()
        .and_then(|(family_id, _)| font_cx.collection.family_name(*family_id))
        .map(str::to_owned)
        .unwrap_or_else(|| "Arimo".to_owned());

    let fake_blob = Blob::new(Arc::new(FAKE_NOTO.to_vec()));
    let fake_blob_id = fake_blob.id();
    let fake_families = font_cx.collection.register_fonts(fake_blob, None);
    // Sanity: the registered family name should be `FAKE_NOTO_FAMILY` (from the
    // generated font's `name` table); we push style spans by that literal name
    // regardless, per Q5 (explicit family span for determinism).
    debug_assert!(
        fake_families
            .first()
            .and_then(|(family_id, _)| font_cx.collection.family_name(*family_id))
            .is_some_and(|n| n == FAKE_NOTO_FAMILY),
        "fake font's registered family name should match FAKE_NOTO_FAMILY"
    );

    (arimo_name, fake_blob_id)
}

/// Build the Parley layout for `text`, with Arimo as the default family and an explicit
/// `FakeNotoEmoji` family span pushed over each emoji `char`'s byte range.
pub(crate) fn build_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ColorBrush>,
    text: &str,
    arimo_family: &str,
    font_size: f32,
    max_advance: Option<f32>,
) -> Layout<ColorBrush> {
    let mut builder: RangedBuilder<'_, ColorBrush> =
        layout_cx.ranged_builder(font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::Brush(ColorBrush {
        color: Color::BLACK,
    }));
    builder.push_default(FontFamily::named(arimo_family));
    builder.push_default(LineHeight::FontSizeRelative(1.3));
    builder.push_default(StyleProperty::FontSize(font_size));

    for (byte_start, c) in text.char_indices() {
        if is_emoji_char(c) {
            let range = byte_start..byte_start + c.len_utf8();
            builder.push(FontFamily::named(FAKE_NOTO_FAMILY), range);
        }
    }

    let mut layout = builder.build(text);
    layout.break_all_lines(max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}
