// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Tests for font shaping style properties.

use std::borrow::Cow;

use crate::test_name;
use crate::util::{ColorBrush, TestEnv, samples};
use parley::setting::Tag;
use parley::style::{FontFeature, FontFeatures, FontVariation, FontVariations, StyleProperty};
use peniko::color::palette::css;

// ============================================================================
// FontVariations Tests
// ============================================================================

#[test]
fn style_variations_weight_axis() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test weight axis (wght) with different values
    for weight in [100.0, 400.0, 700.0, 900.0] {
        let variations = FontVariations::List(Cow::Borrowed(&[FontVariation {
            tag: Tag::new(b"wght"),
            value: weight,
        }]));

        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Arimo")));
        builder.push_default(StyleProperty::FontVariations(variations));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.with_name(&format!("wght_{weight}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_variations_width_axis() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test width axis (wdth) with different values
    for width in [75.0, 100.0, 125.0] {
        let variations = FontVariations::List(Cow::Borrowed(&[FontVariation {
            tag: Tag::new(b"wdth"),
            value: width,
        }]));

        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Roboto Flex")));
        builder.push_default(StyleProperty::FontVariations(variations));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.with_name(&format!("wdth_{width}"))
            .check_layout_snapshot(&layout);
    }
}

#[test]
fn style_variations_multiple_axes() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    use parley::style::FontFamily;

    // Test multiple axes at once
    let variations = FontVariations::List(Cow::Borrowed(&[
        FontVariation {
            tag: Tag::new(b"wght"),
            value: 700.0,
        },
        FontVariation {
            tag: Tag::new(b"wdth"),
            value: 75.0,
        },
    ]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named("Roboto Flex")));
    builder.push_default(StyleProperty::FontVariations(variations));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

#[test]
fn span_metrics_match_run_metrics() {
    use parley::style::FontFamily;
    use parley::{LayoutContext, PositionedLayoutItem};

    use crate::util::{ColorBrush, env::create_font_context};

    let text = "x";
    let mut font_cx = create_font_context();
    let mut layout_cx = LayoutContext::<ColorBrush>::new();

    // In Roboto Flex, the `YTLC` axis varies the x-height.
    let variations: [&[FontVariation]; 2] = [
        &[],
        &[FontVariation {
            tag: Tag::new(b"YTLC"),
            value: 570.0,
        }],
    ];
    let x_heights = variations.map(|variations| {
        let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::named("Roboto Flex")));
        builder.push_default(StyleProperty::FontVariations(FontVariations::List(
            Cow::Borrowed(variations),
        )));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        let line = layout.lines().next().unwrap();
        let Some(PositionedLayoutItem::GlyphRun(glyph_run)) = line.items().next() else {
            panic!("expected a glyph run");
        };
        let span_metrics = glyph_run.span_metrics();
        let font_metrics = glyph_run.run().font_metrics();
        assert_eq!(span_metrics.ascent, font_metrics.ascent);
        assert_eq!(span_metrics.descent, font_metrics.descent);
        assert_eq!(Some(span_metrics.x_height), font_metrics.x_height);
        span_metrics.x_height
    });
    assert_ne!(x_heights[0], x_heights[1]);
}

// ============================================================================
// FontFeatures Tests
// ============================================================================

#[test]
fn style_features_ligatures_on() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LIGATURES;

    // Enable ligatures explicitly
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"liga"),
        value: 1,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_off() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LIGATURES;

    // Disable ligatures
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"liga"),
        value: 0,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_small_caps() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::LATIN;

    // Enable small caps
    let features = FontFeatures::List(Cow::Borrowed(&[FontFeature {
        tag: Tag::new(b"smcp"),
        value: 1,
    }]));

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(features));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_ltr_cluster_details() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "abfi";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.0));

    let line = layout.lines().next().unwrap();
    let item = line.items().next().unwrap();
    let glyph_run = match item {
        parley::PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
        parley::PositionedLayoutItem::InlineBox(_) => unreachable!(),
    };
    let mut last_advance = f32::MAX;
    glyph_run.run().clusters().enumerate().for_each(|(i, c)| {
        match i % 4 {
            // 'a' and 'b' are not ligatures.
            0 | 1 => assert!(!c.is_ligature_start() && !c.is_ligature_continuation()),
            // "f" is the ligature start whose cluster contains the "fi" glyph.
            2 => {
                assert!(c.is_ligature_start());
                assert_eq!(c.glyphs().count(), 1);
                assert_eq!(c.text_range().len(), 1);
                assert_eq!(c.glyphs().next().unwrap().id, 444);
                // The glyph for this ligature lives in the start cluster and should
                // contain the whole ligature's advance.
                assert_eq!(c.glyphs().next().unwrap().advance, c.advance() * 2.0);
            }
            // "i" is the ligature continuation whose cluster shares the advance with
            // the ligature start.
            3 => {
                assert!(c.is_ligature_continuation());
                // A continuation shares its advance with the previous cluster.
                assert_eq!(c.advance(), last_advance);
                assert_eq!(c.text_range().len(), 1);
                assert_eq!(c.glyphs().count(), 0);
            }
            _ => unreachable!(),
        }
        last_advance = c.advance();
    });
    env.check_layout_snapshot(&layout);
}

#[test]
fn style_features_ligatures_rtl_cluster_details() {
    let mut env = TestEnv::new(test_name!(), None);

    let text = "نصٍّ";
    let builder = env.ranged_builder(text);
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.0));

    let line = layout.lines().next().unwrap();
    let item = line.items().next().unwrap();
    let glyph_run = match item {
        parley::PositionedLayoutItem::GlyphRun(glyph_run) => glyph_run,
        parley::PositionedLayoutItem::InlineBox(_) => unreachable!(),
    };
    let mut last_advance = f32::MAX;
    glyph_run.run().clusters().enumerate().for_each(|(i, c)| {
        match i % 4 {
            // "ن" and "ص" are not ligatures.
            0 | 1 => assert!(!c.is_ligature_start() && !c.is_ligature_continuation()),
            // "ّ" is the ligature continuation whose cluster shares the advance with
            // the ligature start.
            2 => {
                assert!(c.is_ligature_continuation());
                assert_eq!(c.text_range().len(), 2);
                assert_eq!(c.glyphs().count(), 0);
            }
            // The first visual character (i.e. the last logical character) is the ligature start.
            3 => {
                assert!(c.is_ligature_start());
                assert_eq!(c.glyphs().count(), 1);
                assert_eq!(c.text_range().len(), 2);
                // The advance should be shared with the previous cluster of the ligature.
                assert_eq!(c.advance(), last_advance);
                // This cluster should contain the one glyph of the ligature whose advance
                // is the sum of the advances of the component clusters.
                assert_eq!(c.glyphs().next().unwrap().advance, c.advance() * 2.0);
            }
            _ => unreachable!(),
        }
        last_advance = c.advance();
    });
    env.check_layout_snapshot(&layout);
}

/// A font feature that doesn't affect the shaping of the letters used in the tests below. It's
/// used to change font features in the middle of a word, without that in itself changing anything.
const UNRELATED_FEATURE: FontFeature = FontFeature {
    tag: Tag::new(b"tnum"),
    value: 1,
};

/// The number of shaped runs on the first line of the layout.
fn run_count(layout: &parley::Layout<ColorBrush>) -> usize {
    layout.lines().next().unwrap().runs().count()
}

/// The number of glyphs of the layout.
fn glyph_count(layout: &parley::Layout<ColorBrush>) -> usize {
    layout
        .lines()
        .flat_map(|line| line.runs())
        .flat_map(|run| run.clusters())
        .map(|cluster| cluster.glyphs().count())
        .sum()
}

/// A change in font features in the middle of a word doesn't break shaping: glyphs on both sides
/// are still kerned.
#[test]
fn style_features_change_keeps_kerning() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "AVAVAV";

    let mut plain = env.ranged_builder(text).build(text);
    plain.break_all_lines(None);

    let mut unkerned = {
        let mut builder = env.ranged_builder(text);
        builder.push_default(StyleProperty::FontFeatures(FontFeatures::List(
            Cow::Borrowed(&[FontFeature {
                tag: Tag::new(b"kern"),
                value: 0,
            }]),
        )));
        builder.build(text)
    };
    unkerned.break_all_lines(None);
    assert!(
        plain.full_width() < unkerned.full_width(),
        "the text should be kerned for this test to be meaningful"
    );

    let mut builder = env.ranged_builder(text);
    for v in [1, 3, 5] {
        builder.push(
            StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&[UNRELATED_FEATURE]))),
            v..v + 1,
        );
        builder.push(StyleProperty::Brush(ColorBrush::new(css::RED)), v..v + 1);
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    assert_eq!(run_count(&layout), 1);
    assert_eq!(layout.full_width(), plain.full_width());

    env.check_layout_snapshot(&layout);
}

/// A ligature forms across a change in font features, as long as the ligature feature itself is
/// enabled for all of its characters.
#[test]
fn style_features_change_keeps_ligature() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "fi";

    let mut builder = env.ranged_builder(text);
    builder.push(
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&[UNRELATED_FEATURE]))),
        1..2,
    );
    builder.push(StyleProperty::Brush(ColorBrush::new(css::RED)), 1..2);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    assert_eq!(run_count(&layout), 1);
    assert_eq!(glyph_count(&layout), 1);

    env.check_layout_snapshot(&layout);
}

/// A ligature doesn't form if the ligature feature is disabled for some of its characters.
#[test]
fn style_features_ligatures_off_for_part_of_ligature() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "fi";

    let mut builder = env.ranged_builder(text);
    builder.push(
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&[FontFeature {
            tag: Tag::new(b"liga"),
            value: 0,
        }]))),
        1..2,
    );
    builder.push(StyleProperty::Brush(ColorBrush::new(css::RED)), 1..2);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    assert_eq!(run_count(&layout), 1);
    assert_eq!(glyph_count(&layout), 2);

    env.check_layout_snapshot(&layout);
}

/// A font feature applies to exactly the characters it's set on, also if those characters are in
/// the middle of a run.
#[test]
fn style_features_ligatures_on_for_range() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "fi fi fi";

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::FontFeatures(FontFeatures::List(
        Cow::Borrowed(&[FontFeature {
            tag: Tag::new(b"liga"),
            value: 0,
        }]),
    )));
    // Only the middle "fi" has ligatures enabled.
    builder.push(
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&[FontFeature {
            tag: Tag::new(b"liga"),
            value: 1,
        }]))),
        3..5,
    );
    builder.push(StyleProperty::Brush(ColorBrush::new(css::RED)), 3..5);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    assert_eq!(run_count(&layout), 1);
    // Two glyphs for the first and last "fi", one for the middle "fi", and two spaces.
    assert_eq!(glyph_count(&layout), 7);

    env.check_layout_snapshot(&layout);
}

/// The range of a font feature can span across runs of different fonts, and can start or end in
/// the middle of a run that isn't the first run of the text.
#[test]
fn style_features_change_across_font_fallback() {
    let mut env = TestEnv::new(test_name!(), None);
    // The Arabic is shaped with a different font than the Latin text following it.
    let text = "fi مرحبا fi fi";
    let last_fi = text.len() - 2..text.len();

    let mut builder = env.ranged_builder(text);
    // Disable ligatures from the "i" of the first "fi", through the Arabic, up to the last "fi".
    builder.push(
        StyleProperty::FontFeatures(FontFeatures::List(Cow::Borrowed(&[FontFeature {
            tag: Tag::new(b"liga"),
            value: 0,
        }]))),
        1..last_fi.start,
    );
    builder.push(
        StyleProperty::Brush(ColorBrush::new(css::RED)),
        1..last_fi.start,
    );
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    let last_fi_glyphs: usize = layout
        .lines()
        .flat_map(|line| line.runs())
        .flat_map(|run| run.clusters())
        .filter(|cluster| cluster.text_range().start >= last_fi.start)
        .map(|cluster| cluster.glyphs().count())
        .sum();
    assert_eq!(last_fi_glyphs, 1, "the last \"fi\" should be a ligature");

    let mut plain = env.ranged_builder(text).build(text);
    plain.break_all_lines(None);
    // Compared to shaping without the feature, the first two "fi" are not ligatures.
    assert_eq!(glyph_count(&layout), glyph_count(&plain) + 2);

    env.check_layout_snapshot(&layout);
}

// ============================================================================
// Locale Tests
// ============================================================================

#[test]
fn style_locale_arabic() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::ARABIC;

    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Locale(Some("ar".parse().unwrap())));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.check_layout_snapshot(&layout);
}

#[test]
fn style_locale_mixed_bidi() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = samples::MIXED_BIDI;

    // Test with Arabic locale
    let mut builder = env.ranged_builder(text);
    builder.push_default(StyleProperty::Locale(Some("ar".parse().unwrap())));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);

    env.with_name("ar").check_layout_snapshot(&layout);

    // Test with English locale
    let mut builder_en = env.ranged_builder(text);
    builder_en.push_default(StyleProperty::Locale(Some("en".parse().unwrap())));
    let mut layout_en = builder_en.build(text);
    layout_en.break_all_lines(None);

    env.with_name("en").check_layout_snapshot(&layout_en);
}

// ============================================================================
// Shaping Split Tests
// ============================================================================

#[test]
fn shaping_split() {
    let mut env = TestEnv::new(test_name!(), None);
    let text = "office";
    let middle_f = 2..3;

    for split in [false, true] {
        let mut builder = env.ranged_builder(text);
        builder.push(
            StyleProperty::Brush(ColorBrush::new(css::RED)),
            middle_f.clone(),
        );
        if split {
            builder.push_shaping_split(middle_f.start);
            builder.push_shaping_split(middle_f.end);
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        env.check_layout_snapshot(&layout);
    }
}
