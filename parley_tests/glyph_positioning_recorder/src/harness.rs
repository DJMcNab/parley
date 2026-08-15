// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The Rust half of the browser harness: mapping a [`Case`] to the payload the page
//! renders.
//!
//! The browser side knows nothing about the [`Case`]/[`Run`] grammar — it applies
//! opaque CSS declaration strings, built by
//! [`container_css`]/[`run_css`] in `parley_glyph_positioning_cases`. Those live there
//! rather than here so the report generator can reproduce this DOM byte-identically;
//! this module is the only thing that turns them into a `WebDriver` payload. See
//! `doc/glyph-positioning-chrome-parity-phase2.md`.
//!
//! The page and script themselves now live in `container/agent/` (`harness.html`,
//! `harness.ts`), served by the in-container agent rather than staged onto a bind
//! mount by this crate — see `doc/glyph-positioning-recorder-agent.md`. They are
//! `include_str!`'d here only so the compile-time pinning tests below still hold the
//! two sides together.

use parley_glyph_positioning_cases::{Case, container_css, run_css};

/// The fixed viewport width, in CSS px, the driver sets once per session.
///
/// See [`VIEWPORT_HEIGHT`].
pub const VIEWPORT_WIDTH: u32 = 1280;

/// The fixed viewport height, in CSS px, the driver sets once per session.
///
/// The viewport is deliberately oversized and never resized per case: generated cases
/// top out around 1000×700px (a case is at most 120 characters at at most 30px, in a
/// container of at most 1000px), so a per-case resize would buy nothing while adding a
/// relayout-settling race to every capture and a per-case variable to a session that
/// is reused across thousands of cases.
///
/// The harness fails loudly rather than silently clipping if content ever exceeds this,
/// so raising it is the correct response to that failure.
pub const VIEWPORT_HEIGHT: u32 = 2048;

/// The upload name the driver `PUT`s a font's bytes to (`crate::agent::AgentClient`),
/// which the page's `@font-face` `src` must name.
#[must_use]
pub fn font_file_name(family: &str) -> String {
    format!("{family}.ttf")
}

/// Builds the payload for the page's `renderAndCapture`.
///
/// ```jsonc
/// {
///   "container": "width:312.5px",
///   "runs": [{ "text": "foo bar", "css": "font-family:\"Roboto\";font-size:17.2341px;…" }]
/// }
/// ```
#[must_use]
pub fn payload(case: &Case) -> serde_json::Value {
    serde_json::json!({
        "container": container_css(case),
        "runs": case
            .runs
            .iter()
            .map(|run| serde_json::json!({ "text": run.text, "css": run_css(run) }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use parley_glyph_positioning_cases::FONTS;

    use super::*;

    /// The page served by the agent at `/harness/harness.html`. Lives here (not at
    /// module scope) because it is `include_str!`'d only for
    /// [`page_declares_every_registered_font`] — a plain non-test build has no use for
    /// the page's own bytes.
    const HARNESS_HTML: &str = include_str!("../container/agent/harness.html");
    /// The browser script the agent bundles and serves as `/harness/harness.js`.
    /// `include_str!`'d only for [`harness_ts_takes_no_skp_dir_parameter`], for the
    /// same reason [`HARNESS_HTML`] lives here.
    const HARNESS_TS: &str = include_str!("../container/agent/harness.ts");

    // The declaration strings themselves are pinned by `container_css_is_stable` and
    // `run_css_is_stable` in `parley_glyph_positioning_cases`, which is where they are
    // defined. What this module owns is their placement in the payload.

    #[test]
    fn payload_covers_every_run_in_order() {
        for seed in 0..64 {
            let case = Case::from_seed(seed);
            let payload = payload(&case);

            assert_eq!(
                payload["container"],
                *container_css(&case),
                "seed {seed}: wrong container CSS"
            );
            let runs = payload["runs"]
                .as_array()
                .expect("`runs` must be an array of runs");
            assert_eq!(
                runs.len(),
                case.runs.len(),
                "seed {seed}: payload must have one entry per run"
            );
            for (entry, run) in runs.iter().zip(&case.runs) {
                assert_eq!(entry["text"], *run.text, "seed {seed}: run text differs");
                assert_eq!(entry["css"], *run_css(run), "seed {seed}: run CSS differs");
            }
        }
    }

    /// The page's `@font-face` rules are static, so they have to agree with [`FONTS`]
    /// and with [`font_file_name`]. Generalising that coupling is what multi-font
    /// support has to do; until then, this test is what holds it together.
    #[test]
    fn page_declares_every_registered_font() {
        assert_eq!(
            FONTS.len(),
            1,
            "harness.html declares one `@font-face` per registered font by hand; adding \
             a font needs a matching rule (or generated CSS), not just a `FONTS` entry"
        );
        for font in FONTS {
            let family = format!("font-family: \"{}\";", font.family);
            assert!(
                HARNESS_HTML.contains(&family),
                "harness.html has no `@font-face` for {}",
                font.family
            );
            let src = format!("url(\"{}\")", font_file_name(font.family));
            assert!(
                HARNESS_HTML.contains(&src),
                "harness.html's `@font-face` for {} does not name the file the driver uploads",
                font.family
            );
        }
    }

    /// `renderAndCapture` gets the SKP directory from `shared.ts`'s `SKP_DIR` now,
    /// not as a parameter — the driver's payload has shrunk to match (see
    /// [`payload`]). This is the compile-time guard that the two stay in sync: if
    /// `harness.ts` ever regains a `skpDir` parameter, this fails rather than the
    /// mismatch surfacing as a confusing runtime argument-count error.
    #[test]
    fn harness_ts_takes_no_skp_dir_parameter() {
        assert!(
            HARNESS_TS.contains("async function renderAndCapture(\n  payload: Payload,"),
            "renderAndCapture's signature changed in a way this pinning test didn't expect"
        );
        assert!(
            !HARNESS_TS.contains("skpDir"),
            "renderAndCapture must not take a skpDir parameter: harness.ts imports SKP_DIR \
             from shared.ts itself, and the driver's payload has no field for it"
        );
    }
}
