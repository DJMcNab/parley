// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Attaching to an already-running recorder container and capturing one case at a time.
//!
//! The driver never runs `docker run` or `docker build`: `container/run.sh` starts the
//! container (see `doc/glyph-positioning-recorder-agent.md`) and this connects to it,
//! configured entirely by [`Config`]'s service URLs. Everything other than the `WebDriver`
//! session itself — font upload, SKP listing/fetch/cleanup, `skp_parser` invocation —
//! goes through [`crate::agent::AgentClient`] rather than a mount or `docker exec`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fantoccini::wd::TimeoutConfiguration;
use fantoccini::{Client, ClientBuilder};
use parley_glyph_positioning_cases::{Case, FONTS, GlyphOutput};

use crate::agent::AgentClient;
use crate::{Result, harness, skp_json};

/// The Chromium binary inside the recorder image.
const CHROME_BINARY: &str = "/opt/chrome-headless-shell-linux64/chrome-headless-shell";

/// The Chromium launch flags every capture depends on.
///
/// `--enable-gpu-benchmarking` exposes `printToSkPicture` at all, `--no-sandbox` is what
/// lets it write files, and `--font-render-hinting=none` is what stops headless Chrome
/// quantizing every advance to a whole pixel — without it exact parity is unreachable
/// (Phase 0 measured this; `text-rendering: geometric-precision`, the flag originally
/// believed to be the lever, has no effect on glyph positions at all). The rest are
/// Phase 2's list: scrollbars or a device-scale factor would move content.
///
/// `--allow-file-access-from-files` is gone: the harness page is served over
/// `http://` by the agent now, not loaded via `file://`.
const CHROME_ARGS: &[&str] = &[
    "--enable-gpu-benchmarking",
    "--no-sandbox",
    "--font-render-hinting=none",
    "--hide-scrollbars",
    "--force-device-scale-factor=1",
    "--disable-dev-shm-usage",
];

/// The script timeout for a capture.
///
/// `WebDriver`'s default for `/execute/async` is 30s. A capture should be far under that,
/// so a low value turns a hang into a prompt error rather than a half-minute stall on
/// every failure in a fuzz run.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);

static NEXT_CAPTURE_ID: AtomicU64 = AtomicU64::new(0);

/// Where the driver and the browser find the container services.
///
/// No container name, no mount pairs, no `docker` on this path at runtime — every
/// non-`WebDriver` transport goes through the agent at [`Config::agent`]. All fields
/// are env-overridable but none is required: the defaults match
/// `container/run.sh`'s published ports.
#[derive(Clone, Debug)]
pub struct Config {
    /// The chromedriver endpoint (`PARLEY_GLYPH_WEBDRIVER`).
    pub webdriver: String,
    /// The in-container agent's base URL (`PARLEY_GLYPH_AGENT`).
    pub agent: String,
    /// The agent URL as seen by Chrome inside the container
    /// (`PARLEY_GLYPH_BROWSER_AGENT`).
    ///
    /// This is normally the same as [`Self::agent`]. Keeping it separate lets an
    /// additional container publish its agent on a different host port while Chrome
    /// continues to use the fixed container-local port.
    pub browser_agent: String,
}

impl Config {
    /// Reads the configuration from the environment. Both variables are optional.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            webdriver: optional_var("PARLEY_GLYPH_WEBDRIVER", "http://127.0.0.1:9515"),
            agent: optional_var("PARLEY_GLYPH_AGENT", "http://127.0.0.1:9516"),
            browser_agent: optional_var("PARLEY_GLYPH_BROWSER_AGENT", "http://127.0.0.1:9516"),
        }
    }
}

fn optional_var(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

/// What one capture produced.
///
/// The raw dump and the SKP bytes are kept alongside the parsed output because a
/// deserializer bug and a genuine parity bug look identical in a position diff alone —
/// the fuzz loop writes both into its artifacts.
#[derive(Debug)]
pub struct Recording {
    /// Chrome's glyph output for the case.
    pub output: GlyphOutput,
    /// `skp_parser`'s raw JSON dump.
    pub json: String,
    /// The captured `.skp`'s raw bytes, fetched from the agent. The next capture
    /// clears it from the container.
    pub skp_bytes: Vec<u8>,
}

/// One long-lived browser session against an already-running recorder container.
#[derive(Debug)]
pub struct Recorder {
    config: Config,
    client: Client,
    agent: AgentClient,
    postscript_name: Option<String>,
}

impl Recorder {
    /// Uploads the registered fonts, opens a `WebDriver` session, navigates to the
    /// agent-served harness page, and runs `initHarness`.
    pub async fn attach(config: Config) -> Result<Self> {
        let capture_id = format!(
            "capture-{}-{}",
            std::process::id(),
            NEXT_CAPTURE_ID.fetch_add(1, Ordering::Relaxed)
        );
        let agent = AgentClient::new(&config.agent, capture_id);
        agent.init_capture()?;
        for font in FONTS {
            agent
                .upload_harness(&harness::font_file_name(font.family), font.bytes)
                .map_err(|error| format!("uploading {}: {error}", font.family))?;
        }

        let capabilities = serde_json::json!({
            "goog:chromeOptions": { "binary": CHROME_BINARY, "args": CHROME_ARGS }
        });
        let capabilities = capabilities
            .as_object()
            .expect("the capabilities blob is a JSON object")
            .clone();

        let client = ClientBuilder::native()
            .capabilities(capabilities)
            .connect(&config.webdriver)
            .await
            .map_err(|error| format!("connecting to {}: {error}", config.webdriver))?;

        let mut recorder = Self {
            config,
            client,
            agent,
            postscript_name: None,
        };
        if let Err(error) = recorder.init_session().await {
            let _ = recorder.client.close().await;
            let _ = recorder.agent.clear_skps();
            return Err(error);
        }
        Ok(recorder)
    }

    /// The one-off session setup: timeouts, viewport, navigation, font loading.
    async fn init_session(&mut self) -> Result<()> {
        self.client
            .update_timeouts(TimeoutConfiguration::new(Some(SCRIPT_TIMEOUT), None, None))
            .await?;

        // Set once, never per case: the corpus tops out around 1000x700px, so resizing
        // per case would buy nothing and add a relayout-settling race to every capture.
        self.client
            .set_window_size(harness::VIEWPORT_WIDTH, harness::VIEWPORT_HEIGHT)
            .await?;

        let url = format!(
            "{}/harness/harness.html",
            self.config.browser_agent.trim_end_matches('/')
        );
        self.client
            .goto(&url)
            .await
            .map_err(|error| format!("navigating to {url}: {error}"))?;

        self.execute_harness(
            "parleyHarness.run(() => parleyHarness.initHarness(), arguments[0])",
            vec![],
        )
        .await
        .map_err(|error| format!("initHarness: {error}"))?;

        Ok(())
    }

    /// Renders `case` and returns what Chrome painted.
    pub async fn capture(&mut self, case: &Case) -> Result<Recording> {
        self.capture_impl(case, true).await
    }

    /// Renders `case` and returns only the parsed glyph output.
    ///
    /// The minimiser calls this path thousands of times and never writes raw capture
    /// artifacts, so it avoids fetching the full SKP bytes from the agent.
    pub async fn capture_output(&mut self, case: &Case) -> Result<GlyphOutput> {
        Ok(self.capture_impl(case, false).await?.output)
    }

    async fn capture_impl(&mut self, case: &Case, fetch_raw_skp: bool) -> Result<Recording> {
        self.agent.clear_skps()?;

        self.execute_harness(
            "parleyHarness.run(
                 () => parleyHarness.renderAndCapture(arguments[0], arguments[1]),
                 arguments[2])",
            vec![
                harness::payload(case),
                serde_json::Value::String(self.agent.capture_id().to_string()),
            ],
        )
        .await
        .map_err(|error| format!("renderAndCapture: {error}"))?;

        // No polling and no size-stability check: `printToSkPicture` is a binding on the
        // renderer main thread and the `/execute/async` callback resolves on that same
        // thread, so the write is complete by the time the driver hears back. Bring-up
        // step B5 is what established this.
        let names = self.agent.list_skps()?;
        // The agent already filters its listing to `layer_*.skp`; this re-checks it
        // rather than trusting it wholesale, matching `skp_json`'s "one honest check"
        // at every trust boundary. `is_layer_skp_name`'s own unit test is what this
        // logic's coverage moved to now that the old host-side filesystem scan is gone.
        for name in &names {
            if !is_layer_skp_name(name) {
                return Err(format!("agent listed a non-layer name: {name}").into());
            }
        }
        let name = match names.as_slice() {
            [name] => name.clone(),
            [] => return Err("capture wrote no layer_*.skp".into()),
            // `printToSkPicture` records no per-layer transform, so several layers
            // cannot be composed back into document space.
            names => {
                return Err(format!(
                    "capture wrote {} layers; the page must paint into exactly one, since \
                     `printToSkPicture` records no per-layer transform to compose them with",
                    names.len()
                )
                .into());
            }
        };

        let skp_bytes = if fetch_raw_skp {
            self.agent.fetch_skp(&name)?
        } else {
            Vec::new()
        };
        let json = self.agent.fetch_commands(&name)?;
        let capture = skp_json::Capture::parse(&json)?;

        // Resolved once per session: the typeface cannot change within one, and
        // re-extracting per capture would double every fuzz iteration's agent round
        // trips. The failure that would additionally catch — whole-document fallback —
        // is already caught by `initHarness` asserting the `@font-face` loaded.
        let postscript_name = match &self.postscript_name {
            Some(name) => name.clone(),
            None => {
                let typeface_bytes = self.agent.fetch_typeface(&name, &capture.typeface_key)?;
                let name = skp_json::typeface_postscript_name(&typeface_bytes)?;
                self.postscript_name = Some(name.clone());
                name
            }
        };

        Ok(Recording {
            output: capture.into_output(&postscript_name),
            json,
            skp_bytes,
        })
    }

    /// Runs a `parleyHarness.run(...)` script and unwraps its result envelope.
    ///
    /// The page never leaves a promise rejected — a rejection would surface here as a
    /// script *timeout* rather than a message — so `ok: false` is the only failure
    /// shape, and its `error` is propagated verbatim.
    async fn execute_harness(
        &self,
        script: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let result = self.client.execute_async(script, args).await?;
        if result["ok"] == serde_json::Value::Bool(true) {
            return Ok(result);
        }
        match result["error"].as_str() {
            Some(error) => Err(error.to_string().into()),
            None => Err(format!("harness returned a malformed envelope: {result}").into()),
        }
    }

    /// Closes the session.
    pub async fn close(self) -> Result<()> {
        let browser_result = self.client.close().await;
        let cleanup_result = self.agent.clear_skps();
        browser_result?;
        cleanup_result
    }
}

/// Captures `case`, recycling the session once if the first attempt fails.
///
/// A `WebDriver` error or a script timeout is assumed to have left the session unusable,
/// so the retry opens a fresh one rather than reusing it. There is no proactive periodic
/// recycling — add it if a run shows drift or leaks, not before.
///
/// Moved here (from `bin/fuzz_loop.rs`) so `src/oracle.rs`'s `ChromeOracle` can share it
/// too; the consecutive-failure counting that decides when a run should give up stays in
/// each caller, since `fuzz_loop` and `ChromeOracle` react to it differently (aborting
/// the loop vs. marking an `OracleFailure` fatal).
pub async fn capture_with_retry(
    recorder: &mut Recorder,
    config: &Config,
    case: &Case,
) -> Result<Recording> {
    let first = match recorder.capture(case).await {
        Ok(recording) => return Ok(recording),
        Err(error) => error,
    };
    eprintln!("capture failed ({first}); recycling the session and retrying once");

    let fresh = Recorder::attach(config.clone())
        .await
        .map_err(|error| -> crate::Error {
            format!("{first}; and the session would not reopen: {error}").into()
        })?;
    let previous = std::mem::replace(recorder, fresh);
    let _ = previous.close().await;

    recorder
        .capture(case)
        .await
        .map_err(|second| format!("{first}; and again after recycling: {second}").into())
}

/// Output-only counterpart to [`capture_with_retry`] for the minimiser.
pub async fn capture_output_with_retry(
    recorder: &mut Recorder,
    config: &Config,
    case: &Case,
) -> Result<GlyphOutput> {
    let first = match recorder.capture_output(case).await {
        Ok(output) => return Ok(output),
        Err(error) => error,
    };
    eprintln!("capture failed ({first}); recycling the session and retrying once");

    let fresh = Recorder::attach(config.clone())
        .await
        .map_err(|error| -> crate::Error {
            format!("{first}; and the session would not reopen: {error}").into()
        })?;
    let previous = std::mem::replace(recorder, fresh);
    let _ = previous.close().await;

    recorder
        .capture_output(case)
        .await
        .map_err(|second| format!("{first}; and again after recycling: {second}").into())
}

/// Whether `name` is one of `printToSkPicture`'s `layer_<n>.skp` outputs.
///
/// Mirrors the agent's own `isLayerSkpName` (`container/agent/agent.ts`) — the agent
/// is the primary enforcement point since it is the one touching the filesystem, but
/// re-checking its listing here means a misbehaving agent surfaces as this error
/// rather than as a confusing downstream parse failure.
fn is_layer_skp_name(name: &str) -> bool {
    name.starts_with("layer_") && name.ends_with(".skp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_layer_skp_names_pass() {
        assert!(is_layer_skp_name("layer_0.skp"));
        assert!(is_layer_skp_name("layer_12.skp"));
        assert!(!is_layer_skp_name("layer_0.skp.tmp"));
        assert!(!is_layer_skp_name("capture.skp"));
    }
}
