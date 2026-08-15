// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Chrome-backed [`Oracle`] the failure minimiser (`bin/minimise.rs`) drives.
//!
//! The minimiser engine itself (`parley_glyph_positioning_cases::minimise`) is sync and
//! oracle-generic, so it can be unit-tested without Chrome; [`ChromeOracle`] is the
//! implementation that actually captures against a running recorder container and
//! compares against Parley, via the same [`layout`]/[`parley_output`] the fuzz loop and
//! CI test use.
//!
//! `bin/minimise.rs`'s `main` is deliberately sync: it owns a current-thread
//! [`tokio::runtime::Runtime`] and [`ChromeOracle`] borrows it, calling
//! [`tokio::runtime::Runtime::block_on`] per uncached capture. `Handle::block_on` cannot
//! drive IO on a current-thread runtime, so the borrowed `&Runtime` (not a `Handle`) is
//! load-bearing here.

use std::collections::HashMap;

use parley::LayoutContext;
use parley_glyph_positioning_cases::{
    Case, GlyphOutput, Mismatch, Oracle, OracleFailure, case_content_key, compare,
};
use parley_glyph_positioning_extract::{font_context, layout, parley_output};

use crate::driver::{Config, Recorder, capture_with_retry};

/// How many consecutive Chrome-capture failures make an [`OracleFailure`] fatal.
///
/// Mirrors `bin/fuzz_loop.rs`'s `MAX_CONSECUTIVE_FAILURES`: past this many failures in a
/// row, the container or the `WebDriver` session is assumed unhealthy, and continuing to
/// mark every further failure as a mere skip would just burn through the rest of the
/// batch one skip message at a time instead of surfacing the real problem.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// A Chrome-backed [`Oracle`]: captures (or reuses a cached capture of) Chrome's output
/// for a candidate [`Case`], lays the same case out with Parley, and compares the two.
///
/// One [`Recorder`] session and one Parley `FontContext`/`LayoutContext` are held for the
/// whole batch of seeds a `minimise` run processes, and Chrome captures are cached by
/// [`case_content_key`] across that whole batch — canonicalised candidates from different
/// seeds converge, so later seeds in a run get cheaper and dedup falls out for free.
#[expect(
    missing_debug_implementations,
    reason = "parley::FontContext and parley::LayoutContext do not implement Debug"
)]
pub struct ChromeOracle<'rt> {
    runtime: &'rt tokio::runtime::Runtime,
    recorder: Recorder,
    config: Config,
    font_cx: parley::FontContext,
    layout_cx: LayoutContext<()>,
    cache: HashMap<String, GlyphOutput>,
    consecutive_failures: u32,
    /// The number of Chrome captures actually performed (cache misses).
    pub captures: u64,
    /// The number of cache hits.
    pub cache_hits: u64,
}

impl<'rt> ChromeOracle<'rt> {
    /// Builds a new oracle around an already-attached `recorder`, borrowing `runtime` to
    /// drive its captures. The Parley-side `FontContext`/`LayoutContext` are built here,
    /// once, via [`font_context`].
    #[must_use]
    pub fn new(runtime: &'rt tokio::runtime::Runtime, recorder: Recorder, config: Config) -> Self {
        Self {
            runtime,
            recorder,
            config,
            font_cx: font_context(),
            layout_cx: LayoutContext::new(),
            cache: HashMap::new(),
            consecutive_failures: 0,
            captures: 0,
            cache_hits: 0,
        }
    }

    /// Returns Chrome's [`GlyphOutput`] for `case`: a cache hit clones the previous
    /// capture for a content-identical case; a miss captures fresh (via
    /// [`capture_with_retry`], recycling the session once on error) and caches only the
    /// [`GlyphOutput`] — the raw JSON dump and `.skp` bytes a fuzz-loop artifact keeps are
    /// not needed once minimisation has moved past a candidate.
    pub fn chrome_output(&mut self, case: &Case) -> crate::Result<GlyphOutput> {
        let key = case_content_key(case);
        if let Some(cached) = self.cache.get(&key) {
            self.cache_hits += 1;
            return Ok(cached.clone());
        }

        let recording =
            self.runtime
                .block_on(capture_with_retry(&mut self.recorder, &self.config, case));
        match recording {
            Ok(recording) => {
                self.consecutive_failures = 0;
                self.captures += 1;
                self.cache.insert(key, recording.output.clone());
                Ok(recording.output)
            }
            Err(error) => {
                self.consecutive_failures += 1;
                Err(error)
            }
        }
    }

    /// Consumes this oracle, returning its [`Recorder`] so the caller can close the
    /// session once the whole batch is done.
    #[must_use]
    pub fn into_recorder(self) -> Recorder {
        self.recorder
    }
}

impl Oracle for ChromeOracle<'_> {
    fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure> {
        let chrome = self.chrome_output(case).map_err(|error| OracleFailure {
            message: error.to_string(),
            fatal: self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES,
        })?;

        let laid_out = layout(case, &mut self.font_cx, &mut self.layout_cx);
        let parley = parley_output(&laid_out);
        Ok(compare(&parley, &chrome))
    }
}
