# Phase One Implementation Plan — Obtain Apple system emoji pixels + metrics in wasm

## 1. Overview / goal

Prove that a Rust application, compiled to `wasm32-unknown-unknown` and running in a
browser, can obtain Apple's **system** color-emoji glyph pixels **and** metrics
**without embedding or serving the Apple emoji font**.

Mechanism (all standard, cross-browser web APIs):

1. Create a 2D canvas, set `ctx.font = "128px 'Apple Color Emoji', ..."`.
2. Use `ctx.measureText(emoji)` to read the **advance width** and the bounding-box
   fields from `TextMetrics` (proving where the ink sits and that nothing is clipped).
3. `ctx.fillText(emoji, x, y)` onto the canvas so the emoji is actually rasterized.
4. Read the pixels back into Rust with `ctx.getImageData(...)` → `ImageData.data()`
   (an RGBA `Uint8ClampedArray`), copied into a Rust `Vec<u8>`.
5. **Prove Rust holds the pixels**: from the Rust side, construct a *new* `ImageData`
   from the Rust-owned buffer (round-tripping the bytes through Rust) and `putImageData`
   it onto a **second, independent canvas**. Also print all captured metrics to the page.

If a validator sees the second canvas showing the same emoji (drawn from Rust-held
bytes) plus a metrics readout with a non-zero advance width and an ink box that fits
inside the drawn region, Phase One is proven.

**Constraints honored:** no Apple font file embedded or served; no changes to Parley
library code; nothing committed. Only new files under `examples/apple_emoji/phase_one`.

## 2. Repository context (what we are reusing)

- Existing wasm/browser precedent: `parley_tests/linebreaking_browser_recorder`
  (Trunk app, `wasm-bindgen` + `web-sys` + `js-sys` + `console_error_panic_hook`,
  `index.html` with `<link data-trunk rel="rust" />`). We mirror its structure.
- Tooling already installed on this machine: `trunk`, `wasm-bindgen`, `wasm-pack`,
  and the `wasm32-unknown-unknown` target (verified).
- Chrome for Testing + matching ChromeDriver are at the paths given in `PLAN.md`.
- Workspace uses an explicit `members = [...]` list (no globs), edition 2024,
  MSRV 1.88, and the Linebender workspace lint set (`[lints] workspace = true`).
- Arimo variable font at
  `parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf` — **not needed in
  Phase One**, noted for later phases.

## 3. Exact file layout

All new, none committed:

```
examples/apple_emoji/phase_one/
├── Cargo.toml
├── index.html                 # Trunk entry (mirrors recorder's index.html)
├── src/
│   └── main.rs                # the wasm app (single file is sufficient)
└── plans/
    └── phase_one_plan.md       # this document
```

Notes:
- `examples/apple_emoji/` itself has **no** `Cargo.toml` (it is just a grouping dir),
  matching how `examples/` groups crates.
- The Trunk build output goes to `examples/apple_emoji/phase_one/dist/` — add
  `/dist` to a local `.gitignore` in that dir (same as recorder). Since nothing is
  committed this is a courtesy only.

## 4. Cargo.toml sketch

```toml
[package]
name = "apple_emoji_phase_one"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[dependencies]
wasm-bindgen = "0.2.100"
wasm-bindgen-futures = "0.4.50"   # only if we await FontFaceSet.ready; see §6
js-sys = "0.3.77"
console_error_panic_hook = "0.1.7"

[dependencies.web-sys]
version = "0.3.77"
features = [
    "Window",
    "Document",
    "Element",
    "HtmlElement",
    "HtmlCanvasElement",
    "CanvasRenderingContext2d",
    "TextMetrics",
    "ImageData",
    "Node",
    "Navigator",
    "console",
    # Optional (font readiness gating, see §6 risk):
    "FontFaceSet",
]

[lints]
workspace = true
```

### Workspace membership decision

The workspace `members` list is explicit (not glob), so a new crate that is neither a
member nor excluded will make `cargo`/`trunk` error with "current package believes it's
in a workspace that does not contain it". Two viable options:

- **Chosen: add `"examples/apple_emoji/phase_one"` to the `members` list in the root
  `Cargo.toml`** (temporary, uncommitted edit). This is exactly how
  `parley_tests/linebreaking_browser_recorder` is handled, and lets `trunk` resolve the
  workspace. It does mean `cargo build --workspace` will also compile this crate for the
  host target — that is fine: `web-sys`/`wasm-bindgen` compile on host, they just don't
  run there.
- **Alternative (if we want zero root-file edits):** add
  `[workspace] exclude = ["examples/apple_emoji/phase_one"]`... but `exclude` must also
  live in the root `Cargo.toml`, so it is *also* a root edit and additionally makes the
  crate its own standalone workspace (its lints/`edition.workspace` would then fail
  because `workspace.package` is no longer inherited). Therefore **member is strictly
  better**; we go with member.

> Validator note: this is the one root-file change. It is a build-enabling edit, not a
> Parley *library* change, and it will not be committed. If the team lead prefers zero
> root edits, the crate can instead be given a self-contained `[package]` (hard-coded
> edition/license, own `[lints]`) plus a `[workspace]` table to detach it — flag this as
> an open question (§9, Q1).

## 5. `index.html` sketch

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Apple system emoji — Phase One pixel/metric capture</title>
  <link data-trunk rel="rust" />
  <style>
    body { font-family: system-ui, sans-serif; margin: 1rem; }
    canvas { border: 1px solid #999; image-rendering: pixelated; margin: 0.5rem; }
    #metrics { white-space: pre; font-family: ui-monospace, monospace; }
    .ok { color: green; font-weight: bold; }
    .bad { color: red; font-weight: bold; }
  </style>
</head>
<body>
  <h1>Apple System Emoji — Phase One</h1>
  <div id="status">Starting…</div>
  <div>
    <div>Canvas A — browser <code>fillText</code>:</div>
    <canvas id="source"></canvas>
  </div>
  <div>
    <div>Canvas B — reconstructed from Rust-held pixels:</div>
    <canvas id="roundtrip"></canvas>
  </div>
  <h2>Metrics (read by Rust)</h2>
  <div id="metrics"></div>
</body>
</html>
```

The two canvases (`#source`, `#roundtrip`) and `#metrics` are the visible proof surface
for the screenshot.

## 6. Canvas / metrics approach (detail)

### Font selection — cross-browser fallback-proofing

The single point of failure is getting the *actual* Apple emoji glyph rather than a
tofu box or a monochrome fallback. Approach:

- Set the font stack to prefer the Apple emoji family with sensible fallbacks:
  `ctx.font = "128px 'Apple Color Emoji', 'Segoe UI Emoji', 'Noto Color Emoji', sans-serif"`.
  On macOS (all of Chrome/Safari/Firefox), `'Apple Color Emoji'` resolves to the
  installed system color-emoji font. The extra families make the *code* portable to
  other OSes for later, but on the target machine Apple's font wins.
- Emoji test string: a **single** emoji code point with an unambiguous colored glyph,
  e.g. `"😀"` (U+1F600). Keep a small array (`😀`, `🎉`, `❤️`, `👍`) so the app renders a
  few and we can eyeball color rendering; the primary proof uses one.
- **Detecting a failed/monochrome/tofu render** (font-fallback-proofing) — three checks,
  any failure marks the run `bad`:
  1. **Advance-width sanity:** `measureText(emoji).width` must be clearly non-zero and
     roughly emoji-square at 128px (expect ~120–140px for a full-width emoji). A value
     near a normal glyph advance or ~0 signals fallback failure.
  2. **Color check:** after `fillText`, scan the captured RGBA buffer for pixels where
     R, G, B differ from each other by more than a threshold (true color, not grayscale)
     AND alpha > 0. A color emoji font produces genuinely colored pixels; a monochrome
     fallback or tofu does not. Report the colored-pixel count.
  3. **Ink presence:** count non-transparent pixels; must be a substantial fraction of
     the glyph box (not empty, not a tiny dot).
- **Font readiness:** system fonts are already installed (no web-font download), so
  `fillText` should work on first paint. To be safe against first-frame races we
  optionally `await document.fonts.ready` (needs `FontFaceSet` + `wasm-bindgen-futures`)
  and/or draw inside a `requestAnimationFrame` / after a `setTimeout(0)` yield before
  capturing. This is cheap insurance; include it.

### Metrics captured from `TextMetrics`

Read and display **all** of these (all are standard; the bounding-box fields are widely
supported in current Chrome/Safari/Firefox):

- `width` — the **advance width** (primary Phase-One deliverable).
- `actualBoundingBoxLeft`, `actualBoundingBoxRight` — horizontal ink extent relative to
  the text origin. Right + left gives ink width.
- `actualBoundingBoxAscent`, `actualBoundingBoxDescent` — vertical ink extent relative
  to the alphabetic baseline.
- `fontBoundingBoxAscent`, `fontBoundingBoxDescent` — font-level vertical extent (useful
  for later baseline work in Phase Two/Four).
- (Also log `emHeightAscent`/`emHeightDescent` if present; treat as optional/best-effort
  since Firefox support has historically lagged — guard with feature-detect and don't
  fail if absent.)

### Clipping detection (prove no clipping)

We size the source canvas and choose the draw origin so the **entire** ink box provably
fits, then verify:

1. Compute the required canvas size from metrics:
   `width = ceil(actualBoundingBoxLeft + actualBoundingBoxRight) + 2*PAD`,
   `height = ceil(actualBoundingBoxAscent + actualBoundingBoxDescent) + 2*PAD`
   with `PAD` (e.g. 8px) margin.
2. Set the origin to `x = PAD + actualBoundingBoxLeft`,
   `y = PAD + actualBoundingBoxAscent`, and set
   `ctx.textBaseline = "alphabetic"` (so the metrics origin semantics match).
3. `fillText`, then `getImageData` the whole canvas.
4. **Edge-scan for clipping:** inspect the outermost 1px border rows/cols of the
   captured buffer. If any border pixel has alpha > 0, ink reached the canvas edge →
   possible clipping → mark `bad`. If the border is fully transparent, the glyph is
   fully contained → **no clipping proven**.
5. Additionally assert the measured ink box (from actual pixels: min/max x,y of
   non-transparent pixels) lies strictly inside the canvas with margin, and that it is
   consistent with the `TextMetrics` bounding box (sanity cross-check between the two
   independent sources of truth).

## 7. Rust ↔ JS boundary design

Keep it thin; all glue via `web-sys`/`js-sys`. No custom `#[wasm_bindgen]` exports are
needed because the app is self-driving from `main()` (like the recorder).

Flow inside `run()` (async, spawned via `wasm_bindgen_futures::spawn_local`):

1. Grab `window` → `document`.
2. (Optional) `JsFuture::from(document.fonts().ready()?).await` to gate on font
   readiness.
3. Get `#source` canvas, cast `dyn_into::<HtmlCanvasElement>()`, size it from a
   preliminary `measureText` (do a first measure on a scratch context, size the canvas,
   then re-measure — canvas state resets on resize).
4. Get `CanvasRenderingContext2d` via `canvas.get_context("2d")?.unwrap().dyn_into()`.
5. Set `ctx.set_font(...)`, `ctx.set_text_baseline("alphabetic")`.
6. `let tm: TextMetrics = ctx.measure_text(emoji)?;` read fields via the typed getters
   (`tm.width()`, `tm.actual_bounding_box_left()`, …).
7. `ctx.fill_text(emoji, origin_x, origin_y)?`.
8. `let image_data: ImageData = ctx.get_image_data(0.0, 0.0, w, h)?;`
   → `let clamped = image_data.data();` (`Clamped<Vec<u8>>`) →
   **copy into an owned `Vec<u8>` in Rust** (`let pixels: Vec<u8> = clamped.0;`). This
   `Vec<u8>` living in Rust *is* "Rust has the pixels".
9. Run the analysis on `pixels` purely in Rust: colored-pixel count, alpha coverage,
   border-clip scan, ink bounding box.
10. **Round-trip proof:** build a fresh `ImageData` from the Rust buffer:
    `ImageData::new_with_u8_clamped_array_and_sh(Clamped(&pixels), w, h)?`, get the
    `#roundtrip` canvas + context, and `ctx2.put_image_data(&new_image_data, 0.0, 0.0)?`.
    (Optionally first apply a trivial Rust-side transform — e.g. draw a 1px magenta frame
    into the buffer around the detected ink box — so the second canvas is *visibly*
    Rust-produced, not just a canvas-to-canvas copy.)
11. Write the metrics + pass/fail verdict into `#metrics` and set `#status`.
12. Expose the same metrics as JSON in a hidden DOM node (e.g.
    `<div id="data-phase-one" hidden>{json}</div>`) so the headless ChromeDriver run can
    scrape exact numbers via `--dump-dom` / `execute_script`, mirroring the recorder's
    headless-extraction pattern. Use `js-sys`/manual string building (no serde needed).

Error handling: `console_error_panic_hook::set_once()` in `main`; `run()` returns
`Result<(), JsValue>` and logs errors via `web_sys::console::error_1`.

## 8. Build / serve / validate procedure

### Build + serve (interactive)

```sh
rustup target add wasm32-unknown-unknown   # already present
cd examples/apple_emoji/phase_one
trunk serve --port 8080
# open http://127.0.0.1:8080 in Chrome/Safari/Firefox on macOS to eyeball
```

For a static build to serve headless:

```sh
cd examples/apple_emoji/phase_one
trunk build            # outputs to ./dist
# serve dist with any static server, e.g.:
python3 -m http.server 8080 --directory dist
```

### Automated validation via ChromeDriver (screenshot capture)

Because ChromeDriver speaks the W3C WebDriver protocol over HTTP, the simplest
dependency-free driver is a small **Python** script (Python 3 ships with macOS) using
`urllib` to hit the ChromeDriver REST endpoints — no `selenium` install required. Store
it at `examples/apple_emoji/phase_one/validate.py` (uncommitted helper) or run inline.

Script outline:

1. Launch ChromeDriver:
   `"/Users/djmcnab/chromedriver/mac_arm-150.0.7871.46/chromedriver-mac-arm64/chromedriver" --port=9515`
   (background).
2. `POST /session` with `goog:chromeOptions.binary` =
   `"/Users/djmcnab/chrome/mac_arm-150.0.7871.46/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing"`
   and args `["--headless=new", "--window-size=800,900", "--force-color-profile=srgb"]`.
   (Try headless first; keep a non-headless fallback — see risk R1.)
3. `POST /session/{id}/url` → `http://127.0.0.1:8080`.
4. Poll `#status` text (via `POST /session/{id}/element` + `/text`, or
   `execute/sync` running `document.getElementById('status').textContent`) until it
   reads the "done"/"PASS" sentinel, with a timeout.
5. Scrape verdict + metrics JSON via
   `execute/sync`: `return document.getElementById('data-phase-one').textContent`.
6. `GET /session/{id}/screenshot` → base64 PNG → decode → write
   `examples/apple_emoji/phase_one/_artifacts/phase_one.png` (uncommitted).
7. `DELETE /session/{id}`, kill ChromeDriver.

Alternatively (even simpler, ignoring the "via ChromeDriver" preference) Chrome for
Testing supports `--headless --screenshot=out.png --window-size=... URL` directly; keep
this as a fallback path. The plan's primary path uses ChromeDriver per PLAN.md's intent.

### What the screenshot / scrape must show (proof surface)

- **Canvas A** (`#source`): the Apple color emoji rendered by the browser.
- **Canvas B** (`#roundtrip`): the *same* emoji reconstructed by `putImageData` from the
  Rust-owned `Vec<u8>` (ideally with the Rust-drawn magenta ink-box frame), visually
  identical to A.
- **Metrics block** (`#metrics`): printed `width` (advance), the four
  actual/font bounding-box values, colored-pixel count, alpha coverage %, border-clip
  result, and a bold **PASS/FAIL** verdict.
- Machine-readable duplicate in `#data-phase-one` for exact assertions.

## 9. Acceptance criteria (for the validator to check)

1. `trunk build` (and `trunk serve`) succeed for the crate on `wasm32-unknown-unknown`
   with no errors; `cargo clippy` is clean under the workspace lint set.
2. Page reaches the "done" status in headless Chrome for Testing within the timeout.
3. `measureText(...).width` (advance) is reported and is non-zero and emoji-plausible
   (~120–140px at 128px font). (Primary deliverable.)
4. All four `actualBoundingBox*` fields are reported and finite.
5. **No embedded/served Apple font:** no `.ttf/.otf/.ttc` asset in the crate; no
   `@font-face`/`FontFace` loading Apple emoji; grep the source + `dist/` to confirm.
6. **Rust holds pixels:** the app copies `getImageData` bytes into a Rust `Vec<u8>` and
   reconstructs Canvas B from that buffer via `put_image_data`; Canvas B is non-empty
   and matches Canvas A. (The magenta frame, if used, is visible → proves Rust touched
   the bytes.)
7. **Color proof:** colored-pixel count > 0 (glyph is genuinely multicolor, i.e. real
   Apple emoji, not a monochrome fallback or tofu).
8. **No clipping:** border-pixel scan finds the canvas edges fully transparent, and the
   pixel-derived ink box lies inside the canvas with margin and is consistent with the
   `TextMetrics` box.
9. Verdict node reads **PASS**; screenshot artifact written and shows Canvas A, Canvas B,
   and the metrics readout.
10. No files committed; only the root `Cargo.toml` `members` edit (uncommitted) plus new
    files under `examples/apple_emoji/phase_one/`. No Parley library files changed.

## 10. Risks / open questions

- **R1 — Headless Chrome + system color emoji (biggest risk).** Headless Chrome on macOS
  *usually* still resolves system fonts including `'Apple Color Emoji'`, but color-emoji
  rendering in headless mode has historically been flaky on some platforms. Mitigation:
  use `--headless=new` (Chrome 150 has the modern headless); if the color check fails in
  headless, fall back to **headed** ChromeDriver (omit `--headless`), which definitely
  has full system font access. Document whichever mode passed. Eyeball validation in
  real Safari/Firefox/Chrome is the human backstop.
- **R2 — `TextMetrics` bounding-box field support.** `actualBoundingBox*` and
  `fontBoundingBox*` are broadly supported in current browsers; `emHeight*` is
  best-effort. Plan feature-detects and never hard-fails on the optional fields.
- **R3 — Firefox advance-width semantics for emoji.** Firefox may report a slightly
  different advance than Chrome/Safari for the same emoji (font metrics differ across
  the OS font version / browser). Phase One only needs *an* advance and *no clipping*,
  not cross-browser numeric equality, so this is acceptable; note it for Phase Two.
- **R4 — Retina/devicePixelRatio.** On a HiDPI Mac, CSS px ≠ device px. For Phase One we
  work entirely in canvas backing-store pixels (canvas width/height attributes = the
  pixel grid we read), so DPR does not corrupt the pixel capture. We do NOT apply a DPR
  transform (keep `getImageData` 1:1 with the backing store). Note for later phases where
  visual sharpness matters.
- **R5 — Workspace membership edit.** See §4: adding to `members` is a root-`Cargo.toml`
  edit (uncommitted, build-enabling only). **Open question Q1:** does the team lead
  prefer this, or a detached self-contained crate (own `[workspace]`)? Recommendation:
  members list.
- **R6 — Emoji selection variability.** Emoji presentation (e.g. `❤️` needs VS16) can
  vary; use unambiguous fully-qualified emoji (`😀`) as the primary proof glyph.

## 11. Summary of the plan's shape

A tiny Trunk/`wasm-bindgen` example crate at `examples/apple_emoji/phase_one`, modeled
on `parley_tests/linebreaking_browser_recorder`, that: sets a canvas font to
`'Apple Color Emoji'`, reads `TextMetrics` (advance + bounding boxes), rasterizes with
`fillText`, pulls the RGBA bytes into a Rust `Vec<u8>` via `getImageData`, analyzes them
in Rust (color/coverage/clipping/ink-box), and re-renders them onto a second canvas via
`putImageData` to prove Rust ownership. Validated by a ChromeDriver Python script that
loads the page, scrapes the metrics/verdict JSON, and captures a screenshot showing both
canvases and the metrics. No Apple font embedded/served, no Parley changes, nothing
committed.
