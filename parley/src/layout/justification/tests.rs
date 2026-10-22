// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::tests::utils::fonts::create_font_context;
use crate::{Alignment, AlignmentOptions, FontFamily, Layout, LayoutContext, StyleProperty};
use std::vec::Vec;

const JUSTIFY_LAST: AlignmentOptions = AlignmentOptions {
    align_when_overflowing: false,
    last_line_alignment: Some(Alignment::Justify),
};

fn layout(text: &str) -> Layout<()> {
    let mut fonts = create_font_context();
    let mut context = LayoutContext::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1., true);
    builder.push_default(FontFamily::from(
        "Roboto, 'Noto Sans CJK SC', 'Noto Kufi Arabic'",
    ));
    // Exercise glyph-run boundaries within shaped runs, including boundaries at spaces.
    if text.len() > 3 && text.is_char_boundary(3) {
        builder.push(StyleProperty::Underline(true), 0..3);
    }
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    let natural_width = layout.data.lines[0].metrics.advance;
    layout.break_all_lines(Some((natural_width * 2.).max(100.)));
    layout
}

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected {expected}, got {actual}"
    );
}

/// The ledger's count and the shares read back by consumers must agree.
fn assert_fills_width(layout: &Layout<()>) {
    for (index, line) in layout.data.lines.iter().enumerate() {
        let justification = layout.data.justification.line(line);
        let runs: Vec<_> = layout.get(index).unwrap().runs().collect();
        let gaps: f32 = runs
            .iter()
            .flat_map(|run| run.line_slice().atoms_start())
            .map(|atom| justification.gaps(&atom).total())
            .sum();
        let free_space = line.metrics.inline_max_coord
            - line.metrics.inline_min_coord
            - line.metrics.indent
            - line.metrics.advance
            + line.metrics.hanging_advance;
        if justification.opportunities() > 0. {
            close(gaps, free_space);
            close(
                runs.iter().map(|run| run.advance()).sum::<f32>() - line.metrics.hanging_advance,
                line.metrics.inline_max_coord - line.metrics.inline_min_coord,
            );
            let right_edge = layout
                .get(index)
                .unwrap()
                .items()
                .map(|item| match item {
                    crate::PositionedLayoutItem::GlyphRun(run) => run.offset() + run.advance(),
                    crate::PositionedLayoutItem::InlineBox(inline_box) => {
                        inline_box.x + inline_box.width
                    }
                })
                .last()
                .unwrap();
            close(
                right_edge - line.metrics.hanging_advance,
                line.metrics.inline_max_coord,
            );
        } else {
            close(gaps, 0.);
        }
    }
}
#[test]
fn ledger_lifecycle() {
    let mut layout = layout("one two three four five");
    assert_eq!(layout.data.justification, JustificationLedger::default());
    for alignment in [Alignment::Start, Alignment::Center] {
        layout.align(alignment, AlignmentOptions::default());
        assert_eq!(layout.data.justification, JustificationLedger::default());
    }
    layout.align(Alignment::Justify, JUSTIFY_LAST);
    let records = layout.data.justification.records.clone();
    let ptr = layout.data.justification.records.as_ptr();
    let capacity = layout.data.justification.records.capacity();
    let lines = layout.data.lines.clone();
    assert_fills_width(&layout);
    for alignment in [
        Alignment::Justify,
        Alignment::Start,
        Alignment::Center,
        Alignment::Justify,
    ] {
        layout.align(
            alignment,
            if alignment == Alignment::Justify {
                JUSTIFY_LAST
            } else {
                AlignmentOptions::default()
            },
        );
        assert!(layout.data.justification.built);
        assert_eq!(layout.data.justification.records, records);
        assert_eq!(layout.data.justification.records.as_ptr(), ptr);
        if alignment != Alignment::Justify {
            assert!(
                layout
                    .data
                    .lines
                    .iter()
                    .all(|line| line.justification_amount == 0.)
            );
        }
    }
    assert_eq!(layout.data.lines, lines);
    layout.break_all_lines(Some(100.));
    assert!(!layout.data.justification.built);
    assert!(layout.data.justification.records.is_empty());
    assert_eq!(layout.data.justification.records.capacity(), capacity);
    assert!(
        layout
            .data
            .lines
            .iter()
            .all(|line| line.justification.is_empty() && line.justification_amount == 0.)
    );
    layout.align(Alignment::Justify, JUSTIFY_LAST);
    assert!(layout.data.justification.built);
    assert_ne!(layout.data.justification.records, records);
    assert_fills_width(&layout);

    layout.clear();
    assert!(!layout.data.justification.built);
    assert!(layout.data.justification.records.is_empty());
    assert_eq!(layout.data.justification.records.capacity(), capacity);
}

#[test]
fn empty_ledger_is_built_and_last_line_can_justify_independently() {
    let mut empty = layout("\u{2060}");
    empty.align(Alignment::Justify, JUSTIFY_LAST);
    assert!(empty.data.justification.built);
    assert!(empty.data.justification.records.is_empty());
    empty.align(Alignment::Start, AlignmentOptions::default());
    assert!(empty.data.justification.built);

    let mut layout = layout("one two");
    layout.align(Alignment::Start, JUSTIFY_LAST);
    assert!(layout.data.justification.built);
    assert_fills_width(&layout);
}

#[test]
fn cursor_can_seek_into_a_line_in_either_direction() {
    let mut layout = layout("one 漢字 two 字典 three");
    layout.align(Alignment::Justify, JUSTIFY_LAST);
    let justification = layout.data.justification.line(&layout.data.lines[0]);
    let atoms: Vec<_> = layout
        .get(0)
        .unwrap()
        .runs()
        .flat_map(|run| run.line_slice().atoms_start())
        .collect();
    for start in 0..atoms.len() {
        for is_rtl in [false, true] {
            let mut cursor = justification.cursor(is_rtl);
            let indices: Vec<_> = if is_rtl {
                (0..=start).rev().collect()
            } else {
                (start..atoms.len()).collect()
            };
            for index in indices {
                let expected = justification.gaps(&atoms[index]);
                close(cursor.gaps(&atoms[index]).total(), expected.total());
                // A peeked atom can be queried again across a glyph-run boundary.
                close(cursor.gaps(&atoms[index]).total(), expected.total());
            }
        }
    }
}

#[test]
fn cj_boundary_ownership() {
    use Share::{Half as H, None as N, Whole as W};
    // Expected byte-indexed owners, derived from visual boundaries, not from the ledger.
    type ExpectedOwner = (usize, Share, Share);
    let cases: &[(&str, &[ExpectedOwner])] = &[
        ("漢字", &[(0, N, H), (3, H, N)]),
        ("漢 字", &[(3, N, W)]),
        ("a漢字b", &[(1, W, H), (4, H, W)]),
        ("漢字典", &[(0, N, H), (3, H, H), (6, H, N)]),
        ("漢　字", &[(0, N, H), (3, H, H), (6, H, N)]),
        ("a　b", &[(1, W, W)]),
        ("漢　", &[]),
        ("漢、。字", &[(0, N, H), (3, H, H), (6, H, H), (9, H, N)]),
        ("漢字 ", &[(0, N, H), (3, H, N)]),
        ("漢\u{a0}字", &[(3, N, W)]),
        ("漢字\u{a0}", &[(0, N, H), (3, H, N)]),
        ("漢\u{2060}字", &[(0, N, H), (6, H, N)]),
        ("\u{2060}", &[]),
        ("abc", &[]),
        ("漢", &[]),
        ("fi漢", &[(2, W, N)]),
    ];
    for &(text, expected) in cases {
        let mut layout = layout(text);
        layout.align(Alignment::Justify, JUSTIFY_LAST);
        let mut actual = Vec::new();
        for run in layout.get(0).unwrap().runs() {
            for atom in run.line_slice().atoms_start() {
                if let Ok(index) = layout
                    .data
                    .justification
                    .records
                    .binary_search_by_key(&atom.shaped_clusters_range().start, |record| {
                        record.cluster
                    })
                {
                    let record = layout.data.justification.records[index];
                    actual.push((
                        atom.characters()[0].text_byte_start as usize,
                        record.before,
                        record.after,
                    ));
                }
            }
        }
        assert_eq!(actual, expected, "{text:?}");
        assert_fills_width(&layout);
        let records = layout.data.justification.records.clone();
        let lines = layout.data.lines.clone();
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout.align(Alignment::Justify, JUSTIFY_LAST);
        assert_eq!(layout.data.justification.records, records, "{text:?}");
        assert_eq!(layout.data.lines, lines, "{text:?}");
        layout.break_all_lines(Some(80.));
        assert!(!layout.data.justification.built);
        layout.align(Alignment::Justify, JUSTIFY_LAST);
        assert_fills_width(&layout);
    }
}

#[test]
fn cj_mixed_direction_fills_width() {
    for text in ["漢مرحبا字", "مرحبا漢字", "漢字مرحبا", "مرحبا"] {
        let mut layout = layout(text);
        layout.align(Alignment::Justify, JUSTIFY_LAST);
        assert_fills_width(&layout);
        if layout.data.justification.records.is_empty() {
            // Zero-opportunity RTL lines remain start aligned (visually right aligned).
            close(layout.data.lines[0].justification_amount, 0.);
            close(
                layout.data.lines[0].metrics.offset,
                layout.data.lines[0].metrics.inline_max_coord
                    - layout.data.lines[0].metrics.advance,
            );
        }
    }
}

#[test]
fn cj_inline_box_is_opaque_and_non_owning() {
    let text = "漢\u{fffc}字";
    let mut fonts = create_font_context();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1., true);
    builder.push_default(FontFamily::named("Noto Sans CJK SC"));
    builder.push_inline_box(crate::InlineBox {
        id: 0,
        kind: crate::InlineBoxKind::InFlow,
        index: 3,
        width: 20.,
        height: 10.,
        baseline: None,
        vertical_align: crate::VerticalAlign::BASELINE,
    });
    let mut layout = builder.build(text);
    layout.break_all_lines(Some(100.));
    layout.align(Alignment::Justify, JUSTIFY_LAST);
    let records = &layout.data.justification.records;
    assert_eq!(records.len(), 2);
    assert_eq!(
        (records[0].before, records[0].after),
        (Share::None, Share::Whole)
    );
    assert_eq!(
        (records[1].before, records[1].after),
        (Share::Whole, Share::None)
    );
    assert_fills_width(&layout);
}

#[test]
fn records_are_eight_bytes() {
    assert_eq!(size_of::<Record>(), 8);
}
