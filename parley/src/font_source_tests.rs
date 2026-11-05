// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{rc::Rc, sync::Arc, vec, vec::Vec};
use core::{cell::Cell, ops::ControlFlow};
use linebender_resource_handle::Blob;
use parlance::{FontVariation, Script, Tag};
use parley_engine::FontMetrics;

use crate::{
    FontData, FontInstance, FontInstanceRef, FontRequest, FontRequestId, FontSource, Layout,
    LayoutContext, Presentation, StyleProperty, SyntheticStyle,
};

const LATIN: &[u8] =
    include_bytes!("../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf");
const ARABIC: &[u8] =
    include_bytes!("../../parley_dev/assets/fonts/noto_fonts/NotoKufiArabic-Regular.otf");
const VARIABLE: &[u8] =
    include_bytes!("../../parley_dev/assets/fonts/roboto_fonts/RobotoFlex-VariableFont.ttf");

fn font(bytes: &'static [u8]) -> FontInstance {
    FontInstance {
        font: FontData::new(Blob::new(Arc::new(bytes)), 0),
        variations: Vec::new(),
        synthetic_style: SyntheticStyle::default(),
    }
}

#[derive(Default)]
struct Source {
    fonts: Vec<FontInstance>,
    metrics: Option<FontInstance>,
    emergency: Option<FontInstance>,
    visited: usize,
    last_resort_calls: usize,
    metric_requests: Vec<FontRequestId>,
    requests: Vec<(
        FontRequestId,
        Script,
        Option<parlance::Language>,
        Presentation,
    )>,
    notifications: Rc<Cell<usize>>,
}

impl FontSource for Source {
    fn candidates(
        &mut self,
        request: &FontRequest<'_>,
        mut visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
    ) {
        self.notifications.set(self.notifications.get() + 1);
        self.requests.push((
            request.id,
            request.script,
            request.language.copied(),
            request.presentation,
        ));
        for font in &self.fonts {
            self.visited += 1;
            if visit(font.as_ref()).is_break() {
                break;
            }
        }
    }

    fn first_available(&mut self, id: FontRequestId) -> Option<FontInstanceRef<'_>> {
        self.metric_requests.push(id);
        self.metrics.as_ref().map(FontInstance::as_ref)
    }

    fn last_resort(&mut self, _: &FontRequest<'_>) -> Option<FontInstanceRef<'_>> {
        self.last_resort_calls += 1;
        self.emergency.as_ref().map(FontInstance::as_ref)
    }
}

fn layout(source: &mut impl FontSource, text: &str) -> Layout<()> {
    LayoutContext::new()
        .ranged_builder(source, text, 1.0, false)
        .build(text)
}

#[test]
fn selects_by_cluster_coverage_and_stops_on_complete_coverage() {
    let latin = font(LATIN);
    let id = latin.font.data.id();
    for text in ["a", "e\u{301}", "\u{e9}"] {
        let mut source = Source {
            fonts: vec![font(ARABIC), latin.clone(), font(LATIN)],
            ..Source::default()
        };
        let layout = layout(&mut source, text);
        assert_eq!(layout.data.shaped_text.fonts()[0].font.data.id(), id);
        assert_eq!(source.visited, 2);
        assert_eq!(source.last_resort_calls, 0);
    }
}

#[test]
fn zero_coverage_keeps_first_candidate_without_last_resort() {
    let first = font(LATIN);
    let id = first.font.data.id();
    let mut source = Source {
        fonts: vec![first, font(LATIN)],
        emergency: Some(font(ARABIC)),
        ..Source::default()
    };
    let layout = layout(&mut source, "\u{13000}");
    assert_eq!(layout.data.shaped_text.fonts()[0].font.data.id(), id);
    assert_eq!(source.visited, 2);
    assert_eq!(source.last_resort_calls, 0);
}

fn without_cmap() -> FontInstance {
    let mut bytes = LATIN.to_vec();
    let count = usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
    let record = (0..count)
        .map(|i| 12 + i * 16)
        .find(|&offset| &bytes[offset..offset + 4] == b"cmap")
        .unwrap();
    bytes[record..record + 4].copy_from_slice(b"cmar");
    let mut instance = font(LATIN);
    instance.font.data = Blob::new(Arc::new(bytes));
    instance
}

#[test]
fn last_resort_is_conditional_and_does_not_require_a_cmap() {
    let emergency = without_cmap();
    let id = emergency.font.data.id();
    let mut source = Source {
        fonts: vec![without_cmap()],
        emergency: Some(emergency),
        ..Source::default()
    };
    let result = layout(&mut source, "a");
    assert_eq!(result.data.shaped_text.fonts()[0].font.data.id(), id);
    assert_eq!(source.last_resort_calls, 1);

    source.fonts.push(font(LATIN));
    source.last_resort_calls = 0;
    let result = layout(&mut source, "a");
    assert_ne!(result.data.shaped_text.fonts()[0].font.data.id(), id);
    assert_eq!(source.last_resort_calls, 0);
}

#[test]
fn no_font_preserves_existing_failure_and_fallback_metrics() {
    let mut source = Source::default();
    let result = layout(&mut source, "abc");
    assert!(result.data.shaped_text.runs().is_empty());
    assert_eq!(source.last_resort_calls, 1);
    assert_eq!(
        result.data.style_metrics[0].ascent,
        FontMetrics::fallback(16.0).ascent
    );
}

#[test]
fn metrics_use_the_no_text_operation_even_for_empty_text() {
    let metrics_font = font(ARABIC);
    let expected = FontMetrics::from_font_instance(&metrics_font.as_ref(), 16.0, &[]).unwrap();
    let mut source = Source {
        fonts: vec![font(LATIN)],
        metrics: Some(metrics_font),
        ..Source::default()
    };
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut source, "", 1.0, false);
    builder.push_default(FontRequestId(42));
    let result = builder.build("");
    assert_eq!(result.data.style_metrics[0].ascent, expected.ascent);
    assert_eq!(source.metric_requests, [FontRequestId(42)]);
    assert!(source.requests.is_empty());
}

#[test]
fn request_contains_resolved_script_language_and_presentation() {
    let mut source = Source {
        fonts: vec![font(LATIN)],
        ..Source::default()
    };
    let text = "ا. ☺\u{fe0e} ☺\u{fe0f}";
    let language = "ar".parse().unwrap();
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut source, text, 1.0, false);
    builder.push_default(FontRequestId(17));
    builder.push_default(StyleProperty::Locale(Some(language)));
    builder.build(text);
    assert!(
        source
            .requests
            .iter()
            .all(|r| r.0 == FontRequestId(17) && r.2 == Some(language))
    );
    assert_eq!(source.requests[0].1, Script::from_bytes(*b"Arab"));
    assert_eq!(source.requests[1].1, Script::from_bytes(*b"Arab"));
    assert!(source.requests.iter().any(|r| r.3 == Presentation::Text));
    assert!(source.requests.iter().any(|r| r.3 == Presentation::Emoji));
}

struct StackCoordinates {
    font: FontInstance,
    weight: f32,
}

impl FontSource for StackCoordinates {
    fn candidates(
        &mut self,
        _: &FontRequest<'_>,
        mut visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
    ) {
        let variations = [
            FontVariation::new(Tag::new(b"wght"), self.weight),
            FontVariation::new(Tag::new(b"GRAD"), 50.0),
            FontVariation::new(Tag::new(b"wdth"), 90.0),
            FontVariation::new(Tag::new(b"opsz"), 24.0),
        ];
        let _ = visit(FontInstanceRef {
            variations: &variations,
            ..self.font.as_ref()
        });
    }

    fn first_available(&mut self, _: FontRequestId) -> Option<FontInstanceRef<'_>> {
        self.font.variations = vec![
            FontVariation::new(Tag::new(b"wght"), self.weight),
            FontVariation::new(Tag::new(b"GRAD"), 50.0),
            FontVariation::new(Tag::new(b"wdth"), 90.0),
            FontVariation::new(Tag::new(b"opsz"), 24.0),
        ];
        Some(self.font.as_ref())
    }
}

#[test]
fn stack_coordinates_are_retained_and_explicit_axes_win() {
    let mut source = StackCoordinates {
        font: font(VARIABLE),
        weight: 100.0,
    };
    let mut context = LayoutContext::<()>::new();
    let text = "Variable";
    let axes = [FontVariation::new(Tag::new(b"wght"), 700.0)];
    let mut builder = context.ranged_builder(&mut source, text, 1.0, false);
    builder.push_default(crate::FontVariations::from(&axes));
    let overridden = builder.build(text);
    assert_eq!(overridden.data.shaped_text.fonts()[0].variations.len(), 4);
    assert_eq!(
        overridden.data.shaped_text.fonts()[0].variations[0].value,
        100.0
    );
    source.weight = 700.0;
    let expected = layout(&mut source, text);
    assert_eq!(
        overridden.data.shaped_text.normalized_coords(),
        expected.data.shaped_text.normalized_coords()
    );
    assert_eq!(
        overridden.data.shaped_text.runs()[0].font_metrics,
        expected.data.shaped_text.runs()[0].font_metrics
    );
    assert_eq!(
        overridden.data.style_metrics[0],
        expected.data.style_metrics[0]
    );
    let blob_id = source.font.font.data.id();
    drop(source);
    assert_eq!(
        overridden.data.shaped_text.fonts()[0].font.data.id(),
        blob_id
    );
    assert!(
        !overridden.data.shaped_text.fonts()[0]
            .font
            .data
            .as_ref()
            .is_empty()
    );
}

#[test]
fn equal_instances_coalesce_but_synthetic_style_splits_runs() {
    struct Styled(FontInstance);
    impl FontSource for Styled {
        fn candidates(
            &mut self,
            r: &FontRequest<'_>,
            mut visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
        ) {
            let _ = visit(FontInstanceRef {
                synthetic_style: SyntheticStyle {
                    embolden: r.id.0 == 1,
                    skew_degrees: None,
                },
                ..self.0.as_ref()
            });
        }
        fn first_available(&mut self, _: FontRequestId) -> Option<FontInstanceRef<'_>> {
            Some(self.0.as_ref())
        }
    }
    let mut source = Styled(font(LATIN));
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut source, "abc", 1.0, false);
    builder.push(FontRequestId(2), 1..2);
    let same = builder.build("abc");
    assert_eq!(same.data.shaped_text.runs().len(), 1);
    let mut builder = context.ranged_builder(&mut source, "abc", 1.0, false);
    builder.push(FontRequestId(1), 1..2);
    let different = builder.build("abc");
    assert_eq!(different.data.shaped_text.runs().len(), 3);
    assert!(
        different
            .data
            .shaped_text
            .fonts()
            .iter()
            .any(|f| f.synthetic_style.embolden)
    );
}

#[cfg(feature = "fontique")]
#[test]
fn fontique_metrics_clear_fallbacks_and_requests_survive_registration() {
    use crate::{FontContext, FontFamily};
    use fontique::{Collection, CollectionOptions, GenericFamily};
    let mut context = FontContext::from_parts(
        Collection::new(CollectionOptions {
            shared: false,
            system_fonts: false,
        }),
        Default::default(),
    );
    let latin = font(LATIN);
    let registered = context
        .collection
        .register_fonts(latin.font.data.clone(), None);
    context
        .collection
        .set_generic_families(GenericFamily::SansSerif, [registered[0].0].into_iter());
    context.collection.append_fallbacks(
        fontique::FallbackKey::new(Script::from_bytes(*b"Latn"), None),
        [registered[0].0].into_iter(),
    );
    let request =
        context.register_request(FontFamily::named("Not registered yet"), Default::default());
    let mut session = context.session();
    let mut yielded = false;
    session.candidates(
        &FontRequest {
            id: request,
            script: Script::from_bytes(*b"Latn"),
            language: None,
            presentation: Presentation::Text,
        },
        |_| {
            yielded = true;
            ControlFlow::Break(())
        },
    );
    assert!(yielded);
    assert!(session.first_available(request).is_none());
    drop(session);
    context.collection.register_fonts(
        latin.font.data,
        Some(fontique::FontInfoOverride {
            family_name: Some("Not registered yet"),
            ..Default::default()
        }),
    );
    assert!(context.session().first_available(request).is_some());
}

#[test]
fn collection_indices_are_distinct_instances() {
    let mut bytes = vec![0_u8; 20];
    bytes[..4].copy_from_slice(b"ttcf");
    bytes[4..8].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&2_u32.to_be_bytes());
    for index in 0..2 {
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let base = bytes.len();
        bytes[12 + index * 4..16 + index * 4].copy_from_slice(&(base as u32).to_be_bytes());
        bytes.extend_from_slice(LATIN);
        let count = usize::from(u16::from_be_bytes(
            bytes[base + 4..base + 6].try_into().unwrap(),
        ));
        for table in 0..count {
            let offset = base + 12 + table * 16 + 8;
            let original = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap());
            bytes[offset..offset + 4].copy_from_slice(&(original + base as u32).to_be_bytes());
        }
    }
    struct CollectionSource(Blob<u8>);
    impl FontSource for CollectionSource {
        fn candidates(
            &mut self,
            r: &FontRequest<'_>,
            mut visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
        ) {
            let _ = visit(FontInstanceRef {
                data: &self.0,
                index: r.id.0 as u32,
                variations: &[],
                synthetic_style: SyntheticStyle::default(),
            });
        }
        fn first_available(&mut self, _: FontRequestId) -> Option<FontInstanceRef<'_>> {
            None
        }
    }
    let mut source = CollectionSource(Blob::new(Arc::new(bytes)));
    let mut context = LayoutContext::<()>::new();
    let mut builder = context.ranged_builder(&mut source, "aa", 1.0, false);
    builder.push(FontRequestId(1), 1..2);
    let result = builder.build("aa");
    let fonts = result.data.shaped_text.fonts();
    assert_eq!(fonts.len(), 2);
    assert_eq!(fonts[0].font.data.id(), fonts[1].font.data.id());
    assert_eq!(fonts[0].font.index, 0);
    assert_eq!(fonts[1].font.index, 1);
}

fn with_cmap(platform: u16, encoding: u16, subtable: &[u8]) -> FontInstance {
    let mut bytes = LATIN.to_vec();
    let count = usize::from(u16::from_be_bytes(bytes[4..6].try_into().unwrap()));
    let record = (0..count)
        .map(|i| 12 + i * 16)
        .find(|&offset| &bytes[offset..offset + 4] == b"cmap")
        .unwrap();
    let offset = bytes.len() as u32;
    bytes[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[record + 12..record + 16].copy_from_slice(&(12 + subtable.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 1]);
    bytes.extend_from_slice(&platform.to_be_bytes());
    bytes.extend_from_slice(&encoding.to_be_bytes());
    bytes.extend_from_slice(&12_u32.to_be_bytes());
    bytes.extend_from_slice(subtable);
    let mut instance = font(LATIN);
    instance.font.data = Blob::new(Arc::new(bytes));
    instance
}

#[test]
fn legacy_and_symbol_cmaps_remain_eligible() {
    let mut mac_roman = vec![0_u8; 262];
    mac_roman[2..4].copy_from_slice(&262_u16.to_be_bytes());
    mac_roman[6 + 0x8e] = 1;
    let format6: Vec<u8> = [6_u16, 12, 0, 97, 1, 1]
        .into_iter()
        .flat_map(u16::to_be_bytes)
        .collect();
    let symbol: Vec<u8> = [
        4_u16,
        32,
        0,
        4,
        4,
        1,
        0,
        0xf061,
        0xffff,
        0,
        0xf061,
        0xffff,
        1_u16.wrapping_sub(0xf061),
        1,
        0,
        0,
    ]
    .into_iter()
    .flat_map(u16::to_be_bytes)
    .collect();
    for (font, text) in [
        (with_cmap(1, 0, &mac_roman), "é"),
        (with_cmap(1, 0, &format6), "a"),
        (with_cmap(3, 0, &symbol), "a"),
    ] {
        let id = font.font.data.id();
        let mut source = Source {
            fonts: vec![font, self::font(LATIN)],
            ..Source::default()
        };
        let result = layout(&mut source, text);
        assert_eq!(result.data.shaped_text.fonts()[0].font.data.id(), id);
        assert_eq!(source.visited, 1);
    }
}

#[test]
fn host_can_queue_loading_and_relayout_without_changing_request_ids() {
    let fallback = font(LATIN);
    let fallback_id = fallback.font.data.id();
    let mut source = Source {
        fonts: vec![fallback],
        ..Source::default()
    };
    let notifications = source.notifications.clone();
    let before = layout(&mut source, "a");
    assert_eq!(notifications.get(), 1);
    assert_eq!(source.requests[0].0, FontRequestId::default());
    let preferred = font(LATIN);
    let preferred_id = preferred.font.data.id();
    source.fonts.insert(0, preferred);
    let after = layout(&mut source, "a");
    assert_eq!(source.requests[1].0, FontRequestId::default());
    assert_eq!(
        after.data.shaped_text.fonts()[0].font.data.id(),
        preferred_id
    );
    assert_eq!(
        before.data.shaped_text.fonts()[0].font.data.id(),
        fallback_id
    );
}
