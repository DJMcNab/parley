// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// Constants shared between `agent.ts` (Deno, serves the HTTP API) and `harness.ts`
// (browser, bundled by `deno bundle` and served as `/harness/harness.js`).
//
// The one entry here today, `SKP_DIR`, is what keeps the container-internal SKP
// directory a single source of truth: the Rust driver never learns this path (see
// `doc/glyph-positioning-recorder-agent.md`) because `harness.ts` imports it directly
// and passes it to `printToSkPicture` itself, and `agent.ts` imports the same constant
// to list/read/clear the same directory.

/**
 * Where `printToSkPicture` writes captures, and where the agent reads/clears them.
 *
 * A plain container-internal directory (created in the Dockerfile, owned by the
 * `parley` user) — no mount, no tmpfs. The host never sees it; only the agent's
 * Session-scoped `/capture/<id>` HTTP routes expose its contents.
 */
export const SKP_DIR = "/skp";
