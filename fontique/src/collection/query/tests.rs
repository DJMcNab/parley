// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::*;
use crate::{CollectionOptions, FontInfoOverride, UnicodeRangeSet};
use alloc::{sync::Arc, vec};

const ROBOTO: &[u8] =
    include_bytes!("../../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf");
const EMOJI: &[u8] = include_bytes!(
    "../../../../parley_dev/assets/fonts/noto_color_emoji/NotoColorEmoji-Subset.ttf"
);

fn collection() -> Collection {
    Collection::new(CollectionOptions {
        shared: false,
        system_fonts: false,
    })
}

fn register(
    collection: &mut Collection,
    data: Blob<u8>,
    name: &str,
    range: Option<&UnicodeRangeSet>,
) -> FamilyId {
    collection.register_fonts(
        data,
        Some(FontInfoOverride {
            family_name: Some(name),
            unicode_range: range,
            ..Default::default()
        }),
    )[0]
    .0
}

#[test]
fn overlapping_registrations_are_yielded_last_first_with_effective_coverage() {
    let mut collection = collection();
    let blob = Blob::new(Arc::new(ROBOTO));
    let family = register(&mut collection, blob.clone(), "Composite", None);
    let digits = UnicodeRangeSet::new(&mut vec![(0x30, 0x39)]);
    register(&mut collection, blob.clone(), "Composite", Some(&digits));
    let later = UnicodeRangeSet::new(&mut vec![(0x35, 0x39)]);
    register(&mut collection, blob.clone(), "Composite", Some(&later));
    let mut cache = SourceCache::default();
    let mut query = collection.query(&mut cache);
    query.set_families([family]);
    let mut indices = vec![];
    query.matches_with(|font| {
        assert_eq!(font.blob.id(), blob.id(), "registrations share font bytes");
        let map = font.charmap().unwrap();
        match font.family.1 {
            2 => {
                assert!(map.map('7').is_some());
                assert!(map.map('3').is_none());
                assert!(map.map('A').is_none());
            }
            1 => {
                assert!(map.map('3').is_some());
                assert!(map.map('A').is_none());
            }
            0 => assert!(map.map('A').is_some()),
            _ => unreachable!(),
        }
        indices.push(font.family.1);
        QueryStatus::Continue
    });
    assert_eq!(indices, [2, 1, 0]);
}

#[test]
fn declared_coverage_does_not_invent_missing_glyphs() {
    let mut collection = collection();
    let range = UnicodeRangeSet::new(&mut vec![(0x41, 0x41), (0x10FFFD, 0x10FFFD)]);
    let family = register(
        &mut collection,
        Blob::new(Arc::new(ROBOTO)),
        "Test",
        Some(&range),
    );
    let mut cache = SourceCache::default();
    let mut query = collection.query(&mut cache);
    query.set_families([family]);
    query.matches_with(|font| {
        let map = font.charmap().unwrap();
        assert!(map.map('A').is_some());
        assert!(map.map('B').is_none());
        assert!(map.map('\u{10FFFD}').is_none());
        QueryStatus::Stop
    });
}

#[test]
fn first_available_font_uses_declared_space_coverage() {
    let mut collection = collection();
    let blob = Blob::new(Arc::new(ROBOTO));
    let family = register(&mut collection, blob.clone(), "Test", None);
    let digits = UnicodeRangeSet::new(&mut vec![(0x30, 0x39)]);
    register(&mut collection, blob, "Test", Some(&digits));
    let mut cache = SourceCache::default();
    let mut query = collection.query(&mut cache);
    query.set_families([family]);
    assert_eq!(query.first_available_font().unwrap().family, (family, 0));
}

#[test]
fn space_need_not_have_an_actual_glyph() {
    let mut collection = collection();
    let space = UnicodeRangeSet::new(&mut vec![(0x20, 0x20)]);
    let family = register(
        &mut collection,
        Blob::new(Arc::new(EMOJI)),
        "Test",
        Some(&space),
    );
    let mut cache = SourceCache::default();
    let mut query = collection.query(&mut cache);
    query.set_families([family]);
    let font = query.first_available_font().unwrap();
    assert!(font.charmap().unwrap().map(' ').is_none());
}

#[test]
fn empty_coverage_is_distinct_from_unrestricted_and_full_is_canonicalized() {
    let mut collection = collection();
    let blob = Blob::new(Arc::new(ROBOTO));
    let empty = UnicodeRangeSet::default();
    let family = register(&mut collection, blob.clone(), "Empty", Some(&empty));
    let full = UnicodeRangeSet::new(&mut vec![(0, 0x10FFFF)]);
    let full_family = register(&mut collection, blob, "Full", Some(&full));
    assert!(
        collection
            .family(full_family)
            .unwrap()
            .default_font()
            .unwrap()
            .unicode_range()
            .is_none()
    );
    let mut cache = SourceCache::default();
    let mut query = collection.query(&mut cache);
    query.set_families([family]);
    assert!(query.first_available_font().is_none());
    query.matches_with(|font| {
        assert!(font.charmap().unwrap().map('A').is_none());
        QueryStatus::Stop
    });
    query.set_families([family, full_family]);
    assert_eq!(query.first_available_font().unwrap().family.0, full_family);
}
