// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::{CharCluster, Coverage, charmap::CharmapIndex};
use crate::{FontInstanceRef, lru_cache::LruCache};

pub struct FontCoverageCache {
    entries: LruCache<(u64, u32), Option<CharmapIndex>>,
}

impl Default for FontCoverageCache {
    fn default() -> Self {
        Self {
            entries: LruCache::new(16),
        }
    }
}

impl core::fmt::Debug for FontCoverageCache {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FontCoverageCache").finish_non_exhaustive()
    }
}

impl FontCoverageCache {
    pub fn coverage(
        &mut self,
        font: FontInstanceRef<'_>,
        cluster: &mut CharCluster,
    ) -> Option<Coverage> {
        let index = self.entries.entry((font.data.id(), font.index), || {
            let face = skrifa::FontRef::from_index(font.data.as_ref(), font.index).ok()?;
            CharmapIndex::new(&face)
        });
        let map = index.as_ref()?.charmap(font.data.as_ref())?;
        Some(cluster.calculate_coverage(|ch| map.map(ch).is_some_and(|glyph| glyph != 0)))
    }
}
