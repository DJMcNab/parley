// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A blocking HTTP client for the in-container agent's transport surface (see
//! `container/agent/agent.ts` and `doc/glyph-positioning-recorder-agent.md`).
//!
//! This replaces the old `docker exec` calls (font/page staging, SKP listing, SKP
//! cleanup, `skp_parser` invocation) with plain HTTP to a container-internal service.
//! It stays synchronous and is called from async contexts exactly as those `docker
//! exec` calls were: a handful of localhost round-trips per capture need no async
//! plumbing, so `ureq` (blocking, no TLS) is used rather than dragging in
//! reqwest/hyper.
//!
//! All parsing stays in [`crate::skp_json`] and `parley_glyph_positioning_cases`; this
//! module only moves bytes.

use std::time::Duration;

use crate::Result;

/// How long a single agent call may take before it is treated as failed.
///
/// Generous for a localhost round-trip plus (for the `/commands` and `/typeface`
/// endpoints) a `skp_parser` invocation; its purpose is to turn a wedged container
/// into a prompt error rather than a silent hang, mirroring why the driver sets its
/// own low `WebDriver` script timeout.
const AGENT_TIMEOUT: Duration = Duration::from_secs(30);

/// A client for the in-container HTTP agent, reused across a session's captures.
#[derive(Debug, Clone)]
pub struct AgentClient {
    /// The agent's base URL (`Config::agent`), with any trailing slash trimmed.
    base: String,
    agent: ureq::Agent,
    capture_id: String,
}

impl AgentClient {
    /// `base` is the agent's own URL, e.g. `http://127.0.0.1:9516`.
    #[must_use]
    pub fn new(base: &str, capture_id: String) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(AGENT_TIMEOUT))
            .build();
        Self {
            base: base.trim_end_matches('/').to_string(),
            agent: config.into(),
            capture_id,
        }
    }

    /// Creates this browser session's private capture directory.
    pub fn init_capture(&self) -> Result<()> {
        let url = self.capture_url();
        self.agent
            .put(&url)
            .send_empty()
            .map_err(|error| format!("PUT {url}: {error}"))?;
        Ok(())
    }

    /// The identifier passed to the browser harness.
    #[must_use]
    pub fn capture_id(&self) -> &str {
        &self.capture_id
    }

    fn capture_url(&self) -> String {
        format!("{}/capture/{}", self.base, self.capture_id)
    }

    /// `PUT /harness/<name>`: uploads `bytes` (fonts, at session start — the harness
    /// HTML and script are served from the mounted agent directory and the startup
    /// bundle, never uploaded).
    pub fn upload_harness(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let url = format!("{}/harness/{name}", self.base);
        self.agent
            .put(&url)
            .send(bytes)
            .map_err(|error| format!("PUT {url}: {error}"))?;
        Ok(())
    }

    /// `DELETE /capture/<id>`: removes this session's previous `layer_*.skp` files.
    pub fn clear_skps(&self) -> Result<()> {
        let url = self.capture_url();
        self.agent
            .delete(&url)
            .call()
            .map_err(|error| format!("DELETE {url}: {error}"))?;
        Ok(())
    }

    /// `GET /capture/<id>`: this session's current SKP names, sorted by the agent.
    pub fn list_skps(&self) -> Result<Vec<String>> {
        let url = self.capture_url();
        let body = self
            .agent
            .get(&url)
            .call()
            .map_err(|error| format!("GET {url}: {error}"))?
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("GET {url}: reading body: {error}"))?;
        serde_json::from_str(&body)
            .map_err(|error| format!("GET {url}: not a JSON array of names: {error}").into())
    }

    /// `GET /capture/<id>/<name>`: the raw captured SKP bytes.
    pub fn fetch_skp(&self, name: &str) -> Result<Vec<u8>> {
        let url = format!("{}/{name}", self.capture_url());
        self.agent
            .get(&url)
            .call()
            .map_err(|error| format!("GET {url}: {error}"))?
            .body_mut()
            .read_to_vec()
            .map_err(|error| format!("GET {url}: reading body: {error}").into())
    }

    /// `GET /capture/<id>/<name>/commands`: `skp_parser`'s JSON command dump.
    pub fn fetch_commands(&self, name: &str) -> Result<String> {
        let url = format!("{}/{name}/commands", self.capture_url());
        let bytes = self.get_or_status_error(self.agent.get(&url), &url)?;
        String::from_utf8(bytes)
            .map_err(|error| format!("GET {url}: response was not UTF-8: {error}").into())
    }

    /// `GET /capture/<id>/<name>/typeface?key=<data-key>`: `skp_parser`'s
    /// stdout, i.e. the serialized typeface bytes.
    ///
    /// `data_key` (e.g. `data/0`) is percent-encoded into the query string here and
    /// decoded back by the agent, which then passes it to `skp_parser` verbatim — it
    /// is never parsed as a path segment on either side.
    pub fn fetch_typeface(&self, name: &str, data_key: &str) -> Result<Vec<u8>> {
        let url = format!("{}/{name}/typeface", self.capture_url());
        let request = self.agent.get(&url).query("key", data_key);
        self.get_or_status_error(request, &url)
    }

    /// Runs `request`, treating a non-2xx response as an error whose body (the
    /// agent's error message — stderr, for the `skp_parser`-backed endpoints) is
    /// folded into the returned `Err`.
    ///
    /// `http_status_as_error` is turned off for exactly this call: `ureq` otherwise
    /// discards the body of an error response, and the whole point of the agent's 5xx
    /// contract is that the body is `skp_parser`'s stderr, surfaced verbatim.
    fn get_or_status_error(
        &self,
        request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
        url: &str,
    ) -> Result<Vec<u8>> {
        let mut response = request
            .config()
            .http_status_as_error(false)
            .build()
            .call()
            .map_err(|error| format!("GET {url}: {error}"))?;
        let status = response.status();
        let bytes = response
            .body_mut()
            .read_to_vec()
            .map_err(|error| format!("GET {url}: reading body: {error}"))?;
        if !status.is_success() {
            let message = String::from_utf8_lossy(&bytes);
            return Err(format!("GET {url} failed ({status}): {}", message.trim()).into());
        }
        Ok(bytes)
    }
}
