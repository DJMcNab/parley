// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for font selection style properties.

use std::borrow::Cow;

use crate::test_name;
use crate::util::{ColorBrush, TestEnv, samples};
use fontique::FontInfoOverride;
use parley::style::StyleProperty;
use parley::{FontWeight, Layout};

/// Helper to build a layout with a single font size applied
fn build_with_font_size(env: &mut TestEnv, text: &str, size: f32) -> Layout<ColorBrush> {
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSize(size));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

// ============================================================================
// FontSize Tests
// ============================================================================

#[test]
fn style_font_size_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    // Test multiple font sizes
    for size in [12.0, 16.0, 24.0, 36.0, 48.0] {
        let layout = build_with_font_size(&mut env, text, size);

        // Snapshot for visual verification
        env.with_name(&format!("size_{size}"))
            .check_layout_snapshot(&layout);
    }
}

// ============================================================================
// FontWeight Tests
// ============================================================================

#[test]
fn style_font_weight_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::FontWeight;
    use parley::style::FontFamily;

    for (weight, name) in [
        (FontWeight::THIN, "thin"),
        (FontWeight::LIGHT, "light"),
        (FontWeight::NORMAL, "normal"),
        (FontWeight::MEDIUM, "medium"),
        (FontWeight::SEMI_BOLD, "semibold"),
        (FontWeight::BOLD, "bold"),
        (FontWeight::BLACK, "black"),
    ] {
        let font_request_8 = env.font_request(
            FontFamily::named("Roboto Flex"),
            fontique::Attributes {
                weight,
                ..Default::default()
            },
        );
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSelection(font_request_8));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.with_name(name).check_layout_snapshot(&layout);
    }
}

#[test]
fn style_font_weight_uses_variable_axis_default() {
    use parley::setting::Tag;
    use parley::style::{FontFamily, FontVariation, FontVariations};

    const FAMILY: &str = "Roboto Flex Declared Thin";
    const TEXT: &str = "Variable weight\nVariable weight";

    let mut env = TestEnv::new(test_name!(), None);
    let roboto_flex = env
        .collection()
        .family_by_name("Roboto Flex")
        .unwrap()
        .default_font()
        .unwrap()
        .load(None)
        .unwrap();
    // Re-register Roboto Flex with a user declared `weight` override.
    let registered = env.collection().register_fonts(
        roboto_flex,
        Some(FontInfoOverride {
            family_name: Some(FAMILY),
            weight: Some(FontWeight::THIN),
            ..Default::default()
        }),
    );
    let font = &registered[0].1[0];
    let weight_axis = font
        .axes()
        .iter()
        .find(|axis| axis.tag.to_be_bytes() == *b"wght")
        .unwrap();
    assert_eq!(font.weight(), FontWeight::THIN);
    assert_eq!(weight_axis.default, FontWeight::NORMAL.value());

    let font_request_6 = env.font_request(
        FontFamily::named(FAMILY),
        fontique::Attributes {
            weight: FontWeight::THIN,
            ..Default::default()
        },
    );
    let mut builder = env.ranged_builder(TEXT);
    builder.push_default(StyleProperty::FontSize(48.0));
    builder.push_default(StyleProperty::FontSelection(font_request_6));

    // The first line relies on Fontique's synthesis. Give the second line the
    // equivalent variation explicitly so the snapshot includes a reference.
    let second_line = TEXT.find('\n').unwrap() + 1;
    builder.push(
        StyleProperty::FontVariations(FontVariations::List(Cow::Borrowed(&[FontVariation {
            tag: Tag::new(b"wght"),
            value: FontWeight::THIN.value(),
        }]))),
        second_line..,
    );

    let mut layout = builder.build(TEXT);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

// ============================================================================
// FontWidth Tests
// ============================================================================

#[test]
fn style_font_width_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::FontWidth;
    use parley::style::FontFamily;

    for (width, name) in [
        (FontWidth::ULTRA_CONDENSED, "ultra_condensed"),
        (FontWidth::CONDENSED, "condensed"),
        (FontWidth::NORMAL, "normal"),
        (FontWidth::EXPANDED, "expanded"),
        (FontWidth::ULTRA_EXPANDED, "ultra_expanded"),
    ] {
        let font_request_4 = env.font_request(
            FontFamily::named("Roboto Flex"),
            fontique::Attributes {
                width,
                ..Default::default()
            },
        );
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSelection(font_request_4));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.with_name(name).check_layout_snapshot(&layout);
    }
}

// ============================================================================
// FontStyle Tests
// ============================================================================

#[test]
fn style_font_style_values() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::FontStyle;
    use parley::style::FontFamily;

    // Note Roboto Flex has a `slnt` axis but no `ital` axis, so italic and oblique get a `slnt`
    // variation.
    for (style, name) in [
        (FontStyle::Normal, "normal"),
        (FontStyle::Italic, "italic"),
        (FontStyle::Oblique(None), "oblique"),
    ] {
        let font_request_2 = env.font_request(
            FontFamily::named("Roboto Flex"),
            fontique::Attributes {
                style,
                ..Default::default()
            },
        );
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontSelection(font_request_2));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.with_name(name).check_layout_snapshot(&layout);
    }
}

// ============================================================================
// FontFamily Tests
// ============================================================================

#[test]
fn style_font_family_named() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test with Roboto (should be available in test fonts)
    let font_request_0 = env.font_request(FontFamily::named("Roboto"), Default::default());
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontSelection(font_request_0));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.with_name("roboto").check_layout_snapshot(&layout);
}
