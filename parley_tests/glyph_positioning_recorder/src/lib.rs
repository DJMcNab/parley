// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Chrome side of the glyph-positioning Chrome-parity comparison: a pinned,
//! containerised Chromium, driven from Rust, whose Skia-recorded draw commands are the
//! oracle Parley's own layout is checked against.
//!
//! This crate is native-only and excluded from the wasm/android CI matrix. Nothing here
//! runs as part of normal CI: the `#[test]` that gates PRs compares Parley against
//! checked-in golden data only.
//!
//! [`harness`] maps a case to its payload; [`agent`] is a blocking HTTP client for the
//! in-container agent (font upload, SKP listing/fetch/cleanup, `skp_parser`
//! invocation); [`driver`] attaches to an already-running container and captures using
//! both; [`skp_json`] turns `skp_parser`'s dump into the shared golden schema. The
//! comparison itself lives in `parley_glyph_positioning_cases`, so the CI test can use
//! it without depending on this crate. [`oracle`] wraps a [`driver::Recorder`] plus the
//! Parley side as a `parley_glyph_positioning_cases::Oracle`, for `bin/minimise.rs`. See
//! `doc/glyph-positioning-recorder-agent.md` and
//! `doc/glyph-positioning-chrome-parity-phase4.md`.

pub mod agent;
pub mod driver;
pub mod harness;
pub mod oracle;
pub mod skp_json;

/// The error type used throughout this crate.
///
/// The workspace has no error crate, and a test harness is not where the first one
/// should arrive; context is added by hand at the points where it helps.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// The result type used throughout this crate. See [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
