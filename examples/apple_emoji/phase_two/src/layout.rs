// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Build the Parley layout: Arimo text with each (single-`char`) emoji replaced by an
//! inline box sized to its Apple-system advance width.

use std::sync::Arc;

use parley::fontique::Blob;
use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, InlineBox, InlineBoxKind, Layout,
    LayoutContext, LineHeight, StyleProperty,
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

/// A single-`char` emoji found in the example text, with its byte offset in the
/// *stripped* (Parley) text.
pub(crate) struct EmojiHit {
    pub ch: char,
    /// Byte offset in the stripped text where the inline box should be inserted.
    pub stripped_byte_index: usize,
}

/// Very small "is this an emoji" check, sufficient for the single-`char` emoji used in
/// `EXAMPLE_TEXT` — this is a documented simplification (ZWJ / multi-scalar emoji are
/// out of scope for Phase Two, see plan §5).
fn is_emoji_char(c: char) -> bool {
    matches!(
        c as u32,
        0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x2190..=0x21FF | 0x2B00..=0x2BFF
    )
}

/// Strip emoji out of `text`, returning the stripped string and the list of emoji hits
/// (with byte offsets valid in the *stripped* string).
pub(crate) fn strip_emoji(text: &str) -> (String, Vec<EmojiHit>) {
    let mut stripped = String::with_capacity(text.len());
    let mut hits = Vec::new();
    for c in text.chars() {
        if is_emoji_char(c) {
            hits.push(EmojiHit {
                ch: c,
                stripped_byte_index: stripped.len(),
            });
        } else {
            stripped.push(c);
        }
    }
    (stripped, hits)
}

/// Register Arimo into `font_cx` and return the family name to style with.
pub(crate) fn register_arimo(font_cx: &mut FontContext) -> String {
    let families = font_cx
        .collection
        .register_fonts(Blob::new(Arc::new(ARIMO.to_vec())), None);
    families
        .first()
        .and_then(|(family_id, _)| font_cx.collection.family_name(*family_id))
        .map(str::to_owned)
        .unwrap_or_else(|| "Arimo".to_owned())
}

/// Box sizing/placement info, computed per emoji from its capture, needed both to build
/// the inline box and later to blit pixels into it.
pub(crate) struct BoxSpec {
    pub id: u64,
    pub stripped_byte_index: usize,
    pub width: f32,
    pub height: f32,
}

/// Build the Parley layout for `stripped_text` with the given inline boxes.
pub(crate) fn build_layout(
    font_cx: &mut FontContext,
    layout_cx: &mut LayoutContext<ColorBrush>,
    stripped_text: &str,
    family_name: &str,
    font_size: f32,
    boxes: &[BoxSpec],
    max_advance: Option<f32>,
) -> Layout<ColorBrush> {
    let mut builder = layout_cx.ranged_builder(font_cx, stripped_text, 1.0, true);
    builder.push_default(StyleProperty::Brush(ColorBrush {
        color: Color::BLACK,
    }));
    builder.push_default(FontFamily::named(family_name));
    builder.push_default(LineHeight::FontSizeRelative(1.3));
    builder.push_default(StyleProperty::FontSize(font_size));

    for b in boxes {
        builder.push_inline_box(InlineBox {
            id: b.id,
            kind: InlineBoxKind::InFlow,
            index: b.stripped_byte_index,
            width: b.width,
            height: b.height,
        });
    }

    let mut layout = builder.build(stripped_text);
    layout.break_all_lines(max_advance);
    layout.align(Alignment::Start, AlignmentOptions::default());
    layout
}
