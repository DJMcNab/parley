// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Declared Unicode coverage of a font registration.

use alloc::{sync::Arc, vec::Vec};
use core::ops::RangeInclusive;

#[derive(Clone, Debug, Default)]
pub struct UnicodeRangeSet(Arc<[(u32, u32)]>);

impl UnicodeRangeSet {
    pub fn new(ranges: &mut Vec<(u32, u32)>) -> Self {
        ranges.retain_mut(|(start, end)| {
            *end = (*end).min(0x10FFFF);
            *start <= *end
        });
        ranges.sort_unstable();
        let mut len = 0;
        for index in 0..ranges.len() {
            let (start, end) = ranges[index];
            if len != 0 && start <= ranges[len - 1].1 + 1 {
                ranges[len - 1].1 = ranges[len - 1].1.max(end);
            } else {
                ranges[len] = (start, end);
                len += 1;
            }
        }
        ranges.truncate(len);
        Self(ranges.as_slice().into())
    }

    /// Returns whether the set contains the codepoint.
    #[inline]
    pub fn contains(&self, codepoint: impl Into<u32>) -> bool {
        let codepoint = codepoint.into();
        let index = self.0.partition_point(|&(start, _)| start <= codepoint);
        index != 0 && codepoint <= self.0[index - 1].1
    }

    /// Returns whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns whether the set covers the entire Unicode code space.
    pub fn is_full(&self) -> bool {
        self.0.as_ref() == [(0, 0x10FFFF)]
    }

    /// Iterates over the sorted, disjoint, non-adjacent inclusive ranges.
    pub fn ranges(&self) -> impl ExactSizeIterator<Item = RangeInclusive<u32>> + '_ {
        self.0.iter().map(|&(start, end)| start..=end)
    }
}

impl PartialEq for UnicodeRangeSet {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for UnicodeRangeSet {}
