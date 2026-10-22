// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::tests::utils::fonts::{FONT_FAMILY_LIST, create_font_context};
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
    builder.push_default(FontFamily::from(FONT_FAMILY_LIST));
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
    let mut layout = layout("one two three four five");
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
fn records_are_eight_bytes() {
    assert_eq!(size_of::<Record>(), 8);
}
