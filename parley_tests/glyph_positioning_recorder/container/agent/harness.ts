// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

// The browser half of the glyph-positioning Chrome-parity harness. See
// `doc/glyph-positioning-chrome-parity-phase2.md` for the original design and
// `doc/glyph-positioning-recorder-agent.md` for how it is served today.
//
// This file is a *dumb renderer*: it knows nothing about the `Case`/`Run` grammar.
// The recorder maps the grammar to CSS declaration strings and this applies them
// opaquely, so the grammar lives in one typed language only.
//
// Bundled by the entrypoint's `deno bundle --platform=browser --format=iife` into a
// single classic script, served by the agent as `/harness/harness.js` and loaded by
// `harness.html` via `<script src="harness.js">` (not `type="module"`), which is why
// the public surface is a global `parleyHarness` object rather than ES exports.
//
// Everything is driven through WebDriver's `/execute/async`, which supplies a
// callback as the final argument:
//
//     parleyHarness.run(() => parleyHarness.initHarness(), arguments[0])
//
//     parleyHarness.run(
//         () => parleyHarness.renderAndCapture(arguments[0]),
//         arguments[1])
//
// Both resolve the callback with `{ok: true, ...}` or `{ok: false, error}` and never
// leave a promise rejected: a rejection surfaces to the driver as a script *timeout*
// rather than as a message.

import { SKP_DIR } from "./shared.ts";

declare global {
  var chrome: { gpuBenchmarking: { printToSkPicture: (dir: string) => void } };
  var parleyHarness: unknown;
}

const CONTAINER_ID = "content";

/** Resolves after the next animation frame. */
function nextFrame(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => resolve());
  });
}

/**
 * Loads every `@font-face` this page declares, failing unless all of them succeed.
 * Call once, after navigation, before the first case.
 *
 * CSS-connected faces load lazily, so an untriggered face sits at `unloaded` forever
 * and a silent fallback to a system font would render every case in the wrong
 * typeface. Loading them up front turns that into a hard failure here rather than a
 * PostScript-name mismatch buried in a later diff.
 */
async function initHarness(): Promise<{ fonts: string[] }> {
  const faces = Array.from(document.fonts);
  if (faces.length === 0) {
    throw new Error("page declares no @font-face rules");
  }
  await Promise.all(faces.map((face) => face.load()));
  await document.fonts.ready;
  const failed = faces.filter((face) => face.status !== "loaded");
  if (failed.length !== 0) {
    throw new Error(
      `font(s) failed to load: ${
        failed.map((face) => `${face.family} (${face.status})`).join(", ")
      }`,
    );
  }
  return { fonts: faces.map((face) => face.family) };
}

interface Payload {
  container: string;
  runs: { text: string; css: string }[];
}

/**
 * Replaces the container with one built from `payload`.
 *
 * Spans are appended with no intervening nodes: under `white-space: pre-wrap` a stray
 * newline between them would render as a space, i.e. a glyph Parley does not have.
 * `textContent` (never `innerHTML`) also means run text needs no escaping.
 */
function buildDom(payload: Payload): HTMLElement {
  const previous = document.getElementById(CONTAINER_ID);
  if (previous !== null) {
    previous.remove();
  }
  const container = document.createElement("div");
  container.id = CONTAINER_ID;
  container.style.cssText = payload.container;
  for (const run of payload.runs) {
    const span = document.createElement("span");
    span.style.cssText = run.css;
    span.textContent = run.text;
    container.appendChild(span);
  }
  document.body.appendChild(container);
  return container;
}

/**
 * Renders one case and captures it, atomically.
 *
 * The two `requestAnimationFrame` waits are what guarantee the new content has been
 * through a compositor commit before capture. If it turns out `printToSkPicture`
 * forces its own frame, they can be dropped — but a stale capture would make a reused
 * session lag by one case, which looks like catastrophic parity failure rather than a
 * harness bug.
 *
 * `SKP_DIR` — the container-internal directory `printToSkPicture` writes into — is
 * imported from `shared.ts` rather than taken as a parameter: the Rust driver never
 * learns this path (see `doc/glyph-positioning-recorder-agent.md`), so this function
 * supplies it itself.
 */
async function renderAndCapture(
  payload: Payload,
): Promise<{ width: number; height: number }> {
  buildDom(payload);

  // Scroll extents rather than the container's own rect, so content hanging past the
  // container width is counted. A clipped SKP silently *drops* glyphs rather than
  // mismatching them, so this must fail loudly.
  const root = document.documentElement;
  const width = root.scrollWidth;
  const height = root.scrollHeight;
  if (width > globalThis.innerWidth || height > globalThis.innerHeight) {
    throw new Error(
      `content ${width}x${height} exceeds viewport ${globalThis.innerWidth}x${globalThis.innerHeight}`,
    );
  }

  await nextFrame();
  await nextFrame();
  globalThis.chrome.gpuBenchmarking.printToSkPicture(SKP_DIR);

  return { width, height };
}

/** Runs `body` and hands the result to `callback` as a result envelope. */
function run(body: () => unknown, callback: (result: object) => void): void {
  Promise.resolve()
    .then(body)
    .then((value) => {
      callback(Object.assign({ ok: true }, value as object));
    })
    .catch((error) => {
      callback({ ok: false, error: String((error && error.stack) || error) });
    });
}

globalThis.parleyHarness = { run, initHarness, renderAndCapture };
