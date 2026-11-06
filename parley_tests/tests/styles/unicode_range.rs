// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Font registration coverage, shaping, and style metrics agree on unicode-range.

use std::sync::Arc;

use fontique::{
    Blob, Collection, CollectionOptions, FontInfoOverride, SourceCache, UnicodeRangeSet,
};
use parley::{FontContext, FontFamily, Layout, LayoutContext, PositionedLayoutItem};

const ROBOTO: &[u8] =
    include_bytes!("../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf");
const ARIMO: &[u8] =
    include_bytes!("../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf");
const EMOJI: &[u8] =
    include_bytes!("../../../parley_dev/assets/fonts/noto_color_emoji/NotoColorEmoji-Subset.ttf");

fn context() -> FontContext {
    FontContext {
        collection: Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        source_cache: SourceCache::default(),
    }
}

fn register(fcx: &mut FontContext, bytes: &'static [u8], range: Option<&UnicodeRangeSet>) -> u64 {
    let blob = Blob::new(Arc::new(bytes));
    let id = blob.id();
    fcx.collection.register_fonts(
        blob,
        Some(FontInfoOverride {
            family_name: Some("Composite"),
            unicode_range: range,
            ..Default::default()
        }),
    );
    id
}

fn layout(fcx: &mut FontContext, text: &str) -> Layout<()> {
    let mut lcx = LayoutContext::new();
    let mut builder = lcx.ranged_builder(fcx, text, 1.0, false);
    builder.push_default(FontFamily::named("Composite"));
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout
}

fn glyph_ids(layout: &Layout<()>) -> Vec<u32> {
    layout
        .lines()
        .flat_map(|line| line.items())
        .flat_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => run.glyphs().map(|glyph| glyph.id).collect(),
            PositionedLayoutItem::InlineBox(_) => Vec::new(),
        })
        .collect()
}

#[test]
fn later_overlapping_face_wins_only_inside_its_range() {
    let mut fcx = context();
    let broad = register(&mut fcx, ROBOTO, None);
    let digits = UnicodeRangeSet::new(&mut vec![(0x30, 0x39)]);
    let restricted = register(&mut fcx, ARIMO, Some(&digits));
    let layout = layout(&mut fcx, "A3B");
    let fonts: Vec<_> = layout
        .lines()
        .flat_map(|line| line.runs())
        .map(|run| run.font().data.id())
        .collect();
    assert_eq!(fonts, [broad, restricted, broad]);
    assert!(glyph_ids(&layout).iter().all(|&id| id != 0));
}

#[test]
fn missing_character_does_not_escape_the_declared_range() {
    let mut fcx = context();
    let digits = UnicodeRangeSet::new(&mut vec![(0x30, 0x39)]);
    register(&mut fcx, ROBOTO, Some(&digits));
    let glyphs = glyph_ids(&layout(&mut fcx, "A3"));
    assert_eq!(glyphs.len(), 2);
    assert_eq!(glyphs[0], 0);
    assert_ne!(glyphs[1], 0);
}

#[test]
fn empty_range_cannot_render_real_characters() {
    let mut fcx = context();
    register(&mut fcx, ROBOTO, Some(&UnicodeRangeSet::default()));
    assert_eq!(glyph_ids(&layout(&mut fcx, "ABC")), [0, 0, 0]);
}

#[test]
fn complete_cluster_beats_later_partial_coverage() {
    let mut fcx = context();
    let broad = register(&mut fcx, ROBOTO, None);
    let base_only = UnicodeRangeSet::new(&mut vec![(0x65, 0x65)]);
    register(&mut fcx, ARIMO, Some(&base_only));
    let layout = layout(&mut fcx, "e\u{301}");
    let fonts: Vec<_> = layout
        .lines()
        .flat_map(|line| line.runs())
        .map(|run| run.font().data.id())
        .collect();
    assert_eq!(fonts, [broad]);
    assert!(glyph_ids(&layout).iter().all(|&id| id != 0));
}

#[test]
fn canonical_composition_can_match_a_declared_range() {
    let mut fcx = context();
    let composed = UnicodeRangeSet::new(&mut vec![(0xE9, 0xE9)]);
    register(&mut fcx, ROBOTO, Some(&composed));
    let reference = glyph_ids(&layout(&mut fcx, "é"));
    assert_eq!(reference.len(), 1);
    assert_ne!(reference[0], 0);
    assert_eq!(glyph_ids(&layout(&mut fcx, "e\u{301}")), reference);
}

#[test]
fn digits_only_override_does_not_change_style_metrics() {
    let mut fcx = context();
    register(&mut fcx, ROBOTO, None);
    let reference = layout(&mut fcx, "A");
    let digits = UnicodeRangeSet::new(&mut vec![(0x30, 0x39)]);
    register(&mut fcx, ARIMO, Some(&digits));
    let actual = layout(&mut fcx, "A");
    assert_eq!(actual.height(), reference.height());
    assert_eq!(
        actual.get(0).unwrap().metrics(),
        reference.get(0).unwrap().metrics()
    );
}

#[test]
fn emoji_variation_selectors_do_not_need_their_own_range() {
    let mut fcx = context();
    register(&mut fcx, EMOJI, None);
    let reference = glyph_ids(&layout(&mut fcx, "\u{270C}\u{FE0F}"));
    let mut fcx = context();
    let base = UnicodeRangeSet::new(&mut vec![(0x270C, 0x270C)]);
    register(&mut fcx, EMOJI, Some(&base));
    let actual = glyph_ids(&layout(&mut fcx, "\u{270C}\u{FE0F}"));
    assert_eq!(actual, reference);
    assert!(actual.iter().any(|&id| id != 0));
}

/// End-to-end diagnostic; run alone in release mode on an idle machine:
/// `cargo test -p parley_tests --release benchmark_unicode_range_layout -- --ignored --nocapture`
#[test]
#[ignore = "manual performance measurement"]
fn benchmark_unicode_range_layout() {
    use std::{hint::black_box, time::Instant};

    struct Case {
        name: &'static str,
        fcx: FontContext,
        lcx: LayoutContext<()>,
        samples: Vec<f64>,
    }
    let names = [
        "unrestricted",
        "explicit full",
        "one range",
        "129 ranges",
        "32 subsets",
        "128 subsets",
    ];
    let mut cases: Vec<_> = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let mut fcx = context();
            let blob = Blob::new(Arc::new(ROBOTO));
            let ranges = match index {
                0 => None,
                1 => Some(UnicodeRangeSet::new(&mut vec![(0, 0x10FFFF)])),
                3 => Some(UnicodeRangeSet::new(
                    &mut core::iter::once((0x20, 0x7E))
                        .chain((0..128).map(|i| (0x1000 + i * 2, 0x1000 + i * 2)))
                        .collect(),
                )),
                _ => Some(UnicodeRangeSet::new(&mut vec![(0x20, 0x7E)])),
            };
            fcx.collection.register_fonts(
                blob.clone(),
                Some(FontInfoOverride {
                    family_name: Some("Composite"),
                    unicode_range: ranges.as_ref(),
                    ..Default::default()
                }),
            );
            let subsets = match index {
                4 => 32,
                5 => 128,
                _ => 1,
            };
            // Put the useful face first so reverse declaration order must scan the
            // irrelevant subsets. All registrations share the same font bytes.
            for subset in 1..subsets {
                let ch = 0x1000 + subset * 2;
                let range = UnicodeRangeSet::new(&mut vec![(ch, ch)]);
                fcx.collection.register_fonts(
                    blob.clone(),
                    Some(FontInfoOverride {
                        family_name: Some("Composite"),
                        unicode_range: Some(&range),
                        ..Default::default()
                    }),
                );
            }
            Case {
                name,
                fcx,
                lcx: LayoutContext::new(),
                samples: Vec::new(),
            }
        })
        .collect();
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(20);
    let build = |case: &mut Case| {
        let mut builder = case
            .lcx
            .ranged_builder(&mut case.fcx, black_box(&text), 1.0, false);
        builder.push_default(FontFamily::named("Composite"));
        let mut layout = builder.build(&text);
        layout.break_all_lines(Some(400.0));
        layout
    };
    let reference = build(&mut cases[0]);
    let expected_ids = glyph_ids(&reference);
    for case in &mut cases {
        let actual = build(case);
        assert_eq!(actual.width(), reference.width());
        assert_eq!(actual.height(), reference.height());
        assert_eq!(glyph_ids(&actual), expected_ids);
    }
    const ITERATIONS: u32 = 500;
    for round in 0..9 {
        for offset in 0..cases.len() {
            let index = (offset + round) % cases.len();
            let case = &mut cases[index];
            let start = Instant::now();
            for _ in 0..ITERATIONS {
                black_box(build(case));
            }
            case.samples
                .push(start.elapsed().as_secs_f64() * 1e9 / f64::from(ITERATIONS));
        }
    }
    let mut baseline = 0.0;
    for case in &mut cases {
        case.samples.sort_by(f64::total_cmp);
        let ns = case.samples[case.samples.len() / 2];
        if baseline == 0.0 {
            baseline = ns;
        }
        println!(
            "{}: {ns:.0} ns/layout ({:+.1}% vs unrestricted)",
            case.name,
            (ns / baseline - 1.0) * 100.0
        );
    }
}
