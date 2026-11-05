// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use alloc::{vec, vec::Vec};
use core::ops::ControlFlow;

use fontique::{Attributes, Collection, Query, QueryFamily, QueryFont, QueryStatus, SourceCache};
use parlance::{FontFamily, FontFamilyName, FontVariation, GenericFamily, Tag};

use crate::{
    FontInstance, FontInstanceRef, FontRequest, FontRequestId, FontSource, Presentation,
    SyntheticStyle,
};

#[derive(Clone, Debug, PartialEq)]
struct Request {
    families: Vec<FontFamilyName<'static>>,
    attributes: Attributes,
}

#[derive(Clone)]
pub struct FontContext {
    pub collection: Collection,
    pub source_cache: SourceCache,
    requests: Vec<Request>,
}

impl Default for FontContext {
    fn default() -> Self {
        Self::from_parts(Collection::default(), SourceCache::default())
    }
}

impl FontContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_parts(collection: Collection, source_cache: SourceCache) -> Self {
        Self {
            collection,
            source_cache,
            requests: vec![Request {
                families: vec![FontFamilyName::Generic(GenericFamily::SansSerif)],
                attributes: Attributes::default(),
            }],
        }
    }

    pub fn request_attributes(&self, id: FontRequestId) -> Option<&Attributes> {
        usize::try_from(id.0)
            .ok()
            .and_then(|index| self.requests.get(index))
            .map(|request| &request.attributes)
    }

    pub fn register_request<'a>(
        &mut self,
        families: impl Into<FontFamily<'a>>,
        attributes: Attributes,
    ) -> FontRequestId {
        let families = match families.into() {
            FontFamily::Source(source) => FontFamilyName::parse_css_list(&source)
                .map_while(Result::ok)
                .map(FontFamilyName::into_owned)
                .collect(),
            FontFamily::Single(family) => vec![family.into_owned()],
            FontFamily::List(families) => families
                .iter()
                .cloned()
                .map(FontFamilyName::into_owned)
                .collect(),
        };
        let request = Request {
            families,
            attributes,
        };
        if let Some(index) = self
            .requests
            .iter()
            .position(|existing| existing == &request)
        {
            return FontRequestId(index as u64);
        }
        let id = FontRequestId(self.requests.len() as u64);
        self.requests.push(request);
        id
    }

    pub fn session(&mut self) -> FontiqueSession<'_> {
        self.source_cache.prune(128, false);
        FontiqueSession {
            query: self.collection.query(&mut self.source_cache),
            requests: &self.requests,
            configured: None,
            selected: None,
            last_resort: None,
        }
    }
}

pub struct FontiqueSession<'a> {
    query: Query<'a>,
    requests: &'a [Request],
    configured: Option<(FontRequestId, Presentation)>,
    selected: Option<FontInstance>,
    last_resort: Option<Option<FontInstance>>,
}

impl FontiqueSession<'_> {
    pub fn request_attributes(&self, id: FontRequestId) -> Option<&Attributes> {
        usize::try_from(id.0)
            .ok()
            .and_then(|index| self.requests.get(index))
            .map(|request| &request.attributes)
    }

    fn configure(&mut self, id: FontRequestId, presentation: Presentation) -> bool {
        let Some(request) = usize::try_from(id.0)
            .ok()
            .and_then(|i| self.requests.get(i))
        else {
            return false;
        };
        if self.configured != Some((id, presentation)) {
            let families = request.families.iter().map(|family| match family {
                FontFamilyName::Named(name) => QueryFamily::Named(name),
                FontFamilyName::Generic(generic) => QueryFamily::Generic(*generic),
            });
            let emoji = (presentation == Presentation::Emoji)
                .then_some(QueryFamily::Generic(GenericFamily::Emoji));
            self.query.set_families(families.chain(emoji));
            self.query.set_attributes(request.attributes);
            self.configured = Some((id, presentation));
        }
        true
    }
}

impl FontSource for FontiqueSession<'_> {
    fn candidates(
        &mut self,
        request: &FontRequest<'_>,
        mut visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
    ) {
        if !self.configure(request.id, request.presentation) {
            return;
        }
        self.query
            .set_fallbacks(fontique::FallbackKey::new(request.script, request.language));
        self.query.matches_with(|font| {
            let mut variations = [FontVariation::new(Tag::new(b"    "), 0.0); 3];
            let instance = instance_ref(font, &mut variations);
            match visit(instance) {
                ControlFlow::Continue(()) => QueryStatus::Continue,
                ControlFlow::Break(()) => QueryStatus::Stop,
            }
        });
    }

    fn first_available(&mut self, id: FontRequestId) -> Option<FontInstanceRef<'_>> {
        if !self.configure(id, Presentation::Text) {
            return None;
        }
        self.query.clear_fallbacks();
        self.selected = self.query.first_available_font().map(|font| {
            let mut variations = [FontVariation::new(Tag::new(b"    "), 0.0); 3];
            instance_ref(&font, &mut variations).into()
        });
        self.selected.as_ref().map(FontInstance::as_ref)
    }

    fn last_resort(&mut self, _request: &FontRequest<'_>) -> Option<FontInstanceRef<'_>> {
        if self.last_resort.is_none() {
            self.query.set_families([
                QueryFamily::Generic(GenericFamily::SansSerif),
                QueryFamily::Generic(GenericFamily::Serif),
                QueryFamily::Generic(GenericFamily::Monospace),
            ]);
            let font = self.query.first_available_font().map(|font| {
                let mut variations = [FontVariation::new(Tag::new(b"    "), 0.0); 3];
                instance_ref(&font, &mut variations).into()
            });
            self.last_resort = Some(font);
            self.configured = None;
        }
        self.last_resort
            .as_ref()?
            .as_ref()
            .map(FontInstance::as_ref)
    }
}

fn instance_ref<'a>(
    font: &'a QueryFont,
    variations: &'a mut [FontVariation; 3],
) -> FontInstanceRef<'a> {
    let settings = font.synthesis.variation_settings();
    for (out, (tag, value)) in variations.iter_mut().zip(settings) {
        *out = FontVariation::new(Tag::new(&tag.to_be_bytes()), *value);
    }
    FontInstanceRef {
        data: &font.blob,
        index: font.index,
        variations: &variations[..settings.len()],
        synthetic_style: SyntheticStyle {
            embolden: font.synthesis.embolden(),
            skew_degrees: font.synthesis.skew(),
        },
    }
}
