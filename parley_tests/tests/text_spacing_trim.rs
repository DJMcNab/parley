// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for `text-spacing-trim`.
//!
//! These use the Noto Sans CJK SC subset, whose fullwidth punctuation has half-width alternates
//! (`halt`). At the font size used, its glyphs are [`FULL`] wide, or [`HALF`] when trimmed. The
//! font doesn't have Latin characters, nor the space: those fall back to Roboto, as a run of
//! their own.

use std::borrow::Cow;

use crate::util::{ColorBrush, env::create_font_context};
use parley::setting::Tag;
use parley::{
    FontFamily, FontFeature, FontFeatures, Layout, LayoutContext, OverflowWrap,
    PositionedLayoutItem, RangedBuilder, StyleProperty, TextSpacingTrim, TextSpacingTrimAdjacent,
    TextSpacingTrimEnd,
};

const FULL: f32 = 16.;
const HALF: f32 = 8.;

/// Only trim adjacent punctuation.
const TRIM_ADJACENT: TextSpacingTrim = TextSpacingTrim {
    adjacent: TextSpacingTrimAdjacent::Trim,
    ..TextSpacingTrim::SPACE_ALL
};
/// Only trim the end of lines that don't fit.
const ALLOW_END: TextSpacingTrim = TextSpacingTrim {
    end: TextSpacingTrimEnd::Allow,
    ..TextSpacingTrim::SPACE_ALL
};

type Property = StyleProperty<'static, ColorBrush>;

fn feature(tag: &[u8; 4], value: u16) -> Property {
    let feature = FontFeature::new(Tag::new(tag), value);
    StyleProperty::FontFeatures(FontFeatures::List(Cow::Owned(vec![feature])))
}

/// Build the layout of `text` with `properties`, and whatever `style` adds for parts of it.
fn build(
    text: &str,
    properties: &[Property],
    style: impl FnOnce(&mut RangedBuilder<'_, ColorBrush>),
) -> Layout<ColorBrush> {
    let mut font_cx = create_font_context();
    let mut layout_cx = LayoutContext::new();
    let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
    builder.push_default(StyleProperty::FontSize(FULL));
    builder.push_default(FontFamily::Source("'Noto Sans CJK SC', Roboto".into()));
    for property in properties {
        builder.push_default(property.clone());
    }
    style(&mut builder);
    builder.build(text)
}

/// The text and advance of each line of `layout`, without hanging whitespace.
fn lines_of(layout: &Layout<ColorBrush>, text: &str) -> Vec<(String, f32)> {
    let lines = layout.lines().map(|line| {
        let metrics = line.metrics();
        let advance = metrics.advance - metrics.hanging_advance;
        (text[line.text_range()].to_owned(), advance)
    });
    lines.collect()
}

/// The advances of the glyphs of `layout`.
fn glyph_advances_of(layout: &Layout<ColorBrush>) -> Vec<f32> {
    let items = layout.lines().flat_map(|line| line.items());
    let runs = items.filter_map(|item| match item {
        PositionedLayoutItem::GlyphRun(run) => Some(run),
        PositionedLayoutItem::InlineBox(_) => None,
    });
    runs.flat_map(|run| run.glyphs().map(|glyph| glyph.advance).collect::<Vec<_>>())
        .collect()
}

/// The lines of `text` with `properties`, broken to be at most `width`; see [`lines_of`].
fn lines(text: &str, width: f32, properties: &[Property]) -> Vec<(String, f32)> {
    let mut layout = build(text, properties, |_| {});
    layout.break_all_lines(Some(width));
    lines_of(&layout, text)
}

/// The advances of the glyphs of `text` with `properties`, on a single line.
fn glyph_advances(text: &str, properties: &[Property]) -> Vec<f32> {
    let mut layout = build(text, properties, |_| {});
    layout.break_all_lines(None);
    glyph_advances_of(&layout)
}

fn line(text: &str, advance: f32) -> (String, f32) {
    (text.to_owned(), advance)
}

#[test]
fn text_spacing_trim_adjacent_pairs() {
    // The inner of two characters that face the same way is trimmed.
    assert_eq!(
        glyph_advances("「「水」」", &[TextSpacingTrim::NORMAL.into()]),
        [FULL, HALF, FULL, HALF, FULL]
    );
    // A closing character followed by an opening one trims the opening one.
    assert_eq!(
        glyph_advances("水」「水", &[TRIM_ADJACENT.into()]),
        [FULL, FULL, HALF, FULL]
    );
    // Dots are closing punctuation and the ideographic space is a middle dot.
    assert_eq!(
        glyph_advances("。」\u{3000}（", &[TRIM_ADJACENT.into()]),
        [HALF, HALF, FULL, HALF]
    );
    // Punctuation is fullwidth in Traditional Chinese, so it isn't trimmed, but still trims.
    let traditional = StyleProperty::Locale(Some("zh-Hant".parse().unwrap()));
    assert_eq!(
        glyph_advances("。」。", &[TRIM_ADJACENT.into(), traditional]),
        [FULL, HALF, FULL]
    );
}

#[test]
fn text_spacing_trim_space_all_is_the_default() {
    for text in ["「「水」」", "水」「水", "。」\u{3000}（"] {
        let spaced = glyph_advances(text, &[TextSpacingTrim::SPACE_ALL.into()]);
        assert!(spaced.iter().all(|&advance| advance == FULL));
        assert_eq!(glyph_advances(text, &[]), spaced);
        assert_eq!(glyph_advances(text, &[ALLOW_END.into()]), spaced);
    }
}

#[test]
fn text_spacing_trim_across_fonts() {
    // Narrow punctuation of another font trims fullwidth punctuation it encloses.
    let advances = glyph_advances("(「水」)", &[TRIM_ADJACENT.into()]);
    assert_eq!(advances[1..4], [HALF, FULL, HALF]);
    assert_eq!(
        [advances[0], advances[4]],
        glyph_advances("()", &[])[..],
        "narrow punctuation is never trimmed"
    );

    // The curly quotes are fullwidth in the CJK font, and trimmed like other punctuation...
    let normal: Property = TextSpacingTrim::NORMAL.into();
    assert_eq!(
        glyph_advances("““水””", std::slice::from_ref(&normal)),
        [FULL, HALF, FULL, HALF, FULL]
    );
    // ... but Roboto has no half-width alternates for them, so there they don't change.
    let roboto: Property = FontFamily::named("Roboto").into();
    assert_eq!(
        glyph_advances("““a””", &[roboto.clone(), normal]),
        glyph_advances("““a””", &[roboto])
    );
}

#[test]
fn text_spacing_trim_and_style_features() {
    // Trimming wins over the style turning the feature off...
    assert_eq!(
        glyph_advances("「「水", &[TRIM_ADJACENT.into(), feature(b"halt", 0)]),
        [FULL, HALF, FULL]
    );
    // ... and leaves alone what the style turns on, including for `space-all`.
    assert_eq!(
        glyph_advances("「「水", &[TRIM_ADJACENT.into(), feature(b"halt", 1)]),
        [HALF, HALF, FULL]
    );
    assert_eq!(
        glyph_advances("「「水", &[feature(b"halt", 1)]),
        [HALF, HALF, FULL]
    );

    // Other features, here of a style that starts inside the pair, still apply.
    let text = "（（fi";
    let mut layout = build(text, &[TRIM_ADJACENT.into()], |builder| {
        builder.push(feature(b"liga", 0), "（".len()..text.len());
    });
    layout.break_all_lines(None);
    let advances = glyph_advances_of(&layout);
    assert_eq!(advances[..2], [FULL, HALF]);
    assert_eq!(advances.len(), 4, "\"fi\" is not a ligature");
    assert_eq!(glyph_advances(text, &[]).len(), 3, "\"fi\" is a ligature");
}

#[test]
fn text_spacing_trim_pair_split_at_line_start() {
    // The opening character is no longer trimmed without the closing one before it.
    let trim: Property = TRIM_ADJACENT.into();
    assert_eq!(
        lines("水」「水", 3. * FULL, std::slice::from_ref(&trim)),
        [line("水」", 2. * FULL), line("「水", 2. * FULL)]
    );
    // What reshaping the line's start adds is taken into account when fitting the line.
    assert_eq!(
        lines("水」「水水", 2.5 * FULL, std::slice::from_ref(&trim)),
        [
            line("水」", 2. * FULL),
            line("「水", 2. * FULL),
            line("水", FULL)
        ]
    );
    // Without the split, the pair stays trimmed.
    assert_eq!(
        lines("水」「水", 3.5 * FULL, std::slice::from_ref(&trim)),
        [line("水」「水", 3.5 * FULL)]
    );
    // Undoing the trim restores the style's own setting.
    assert_eq!(
        lines("水」「水", 1.5 * FULL, &[trim, feature(b"halt", 1)]),
        [line("水」", 1.5 * FULL), line("「水", 1.5 * FULL)]
    );
}

#[test]
fn text_spacing_trim_pair_split_at_line_end() {
    // Lines can only break inside a pair that trims its first character in an emergency.
    let properties = [TRIM_ADJACENT.into(), OverflowWrap::Anywhere.into()];
    // The closing character is no longer trimmed without the one after it.
    assert_eq!(
        lines("水」」水", 2. * FULL, &properties),
        [line("水」", 2. * FULL), line("」水", 2. * FULL)]
    );
    // Trimmed it would fit, but it isn't trimmed, so the line breaks before it instead.
    assert_eq!(
        lines("水」」水", 1.5 * FULL, &properties),
        [line("水", FULL), line("」」", 1.5 * FULL), line("水", FULL)]
    );
    // With `allow-end`, it may be trimmed for being at the end of the line.
    let properties = [
        TextSpacingTrim::NORMAL.into(),
        OverflowWrap::Anywhere.into(),
    ];
    assert_eq!(
        lines("水」」水", 1.5 * FULL, &properties),
        [line("水」", 1.5 * FULL), line("」", FULL), line("水", FULL)]
    );
}

#[test]
fn text_spacing_trim_allow_end() {
    let allow_end = [ALLOW_END.into()];
    let text = "水水」水";
    // The closing character fits, so it isn't trimmed.
    assert_eq!(
        lines(text, 3. * FULL, &allow_end),
        [line("水水」", 3. * FULL), line("水", FULL)]
    );
    // It only fits when trimmed.
    assert_eq!(
        lines(text, 2.5 * FULL, &allow_end),
        [line("水水」", 2.5 * FULL), line("水", FULL)]
    );
    // It doesn't fit even when trimmed. Lines can't break before it.
    assert_eq!(
        lines(text, 2.25 * FULL, &allow_end),
        [line("水", FULL), line("水」", 2. * FULL), line("水", FULL)]
    );
    // Without `allow-end`, it isn't trimmed to fit.
    assert_eq!(
        lines(text, 2.5 * FULL, &[TRIM_ADJACENT.into()]),
        [line("水", FULL), line("水」", 2. * FULL), line("水", FULL)]
    );

    // It is not trimmed if the line can't end after it...
    assert_eq!(
        lines("水」。水", 1.5 * FULL, &allow_end),
        [line("水」。", 3. * FULL), line("水", FULL)]
    );
    // ... but is at the end of the text, or before a forced break.
    assert_eq!(
        lines("水水」\n水」", 2.5 * FULL, &allow_end),
        [line("水水」\n", 2.5 * FULL), line("水」", 2. * FULL)]
    );
    assert_eq!(
        lines("水水」", 2.5 * FULL, &allow_end),
        [line("水水」", 2.5 * FULL)]
    );
}

#[test]
fn text_spacing_trim_allow_end_before_hanging_whitespace() {
    let allow_end = [ALLOW_END.into()];
    // The ideographic space is in the closing character's run, the space is in another font.
    for space in ["\u{3000}", " ", " \u{3000} "] {
        let text = format!("水水」{space}水");
        let first_line = format!("水水」{space}");
        assert_eq!(
            lines(&text, 3. * FULL, &allow_end),
            [line(&first_line, 3. * FULL), line("水", FULL)]
        );
        assert_eq!(
            lines(&text, 2.5 * FULL, &allow_end),
            [line(&first_line, 2.5 * FULL), line("水", FULL)]
        );
        assert_eq!(lines(&text, 2.25 * FULL, &allow_end)[0], line("水", FULL));
    }
}

#[test]
fn text_spacing_trim_is_decided_per_layout_width() {
    let text = "水水」水";
    let mut layout = build(text, &[ALLOW_END.into()], |_| {});
    for width in [2.5 * FULL, 3. * FULL, 2.5 * FULL] {
        layout.break_all_lines(Some(width));
        assert_eq!(
            lines_of(&layout, text),
            [line("水水」", width), line("水", FULL)]
        );
    }
}
