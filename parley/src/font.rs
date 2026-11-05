// Copyright 2021 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use core::ops::ControlFlow;

use parlance::{Language, Script};
pub use parley_engine::{FontInstance, FontInstanceRef, SyntheticStyle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FontRequestId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    Text,
    Emoji,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontRequest<'a> {
    pub id: FontRequestId,
    pub script: Script,
    pub language: Option<&'a Language>,
    pub presentation: Presentation,
}

pub trait FontSource {
    fn candidates(
        &mut self,
        request: &FontRequest<'_>,
        visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
    );

    fn first_available(&mut self, id: FontRequestId) -> Option<FontInstanceRef<'_>>;

    fn last_resort(&mut self, _request: &FontRequest<'_>) -> Option<FontInstanceRef<'_>> {
        None
    }
}

impl<F: FontSource + ?Sized> FontSource for &mut F {
    fn candidates(
        &mut self,
        request: &FontRequest<'_>,
        visit: impl FnMut(FontInstanceRef<'_>) -> ControlFlow<()>,
    ) {
        (**self).candidates(request, visit);
    }

    fn first_available(&mut self, id: FontRequestId) -> Option<FontInstanceRef<'_>> {
        (**self).first_available(id)
    }

    fn last_resort(&mut self, request: &FontRequest<'_>) -> Option<FontInstanceRef<'_>> {
        (**self).last_resort(request)
    }
}
