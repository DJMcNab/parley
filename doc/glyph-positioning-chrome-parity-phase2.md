# Phase 2 — browser harness

Implementation spec for Phase 2 of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md).
Read the parent plan first; this file is the authority on what Phase 2 builds.

Phase 2 delivers the harness page **and** the Rust half that feeds it. Nothing
here runs until Phase 4 — see "Verification" below.

**Status: implemented 2026-08-12.** Everything below is built, and its five unit
tests pass; `cargo clippy --workspace --all-targets --all-features` is clean. No
part of it has been executed in a browser. Treat this file as the contract Phase
4 codes against, not as a to-do list.

## Deliverables

A new crate `parley_glyph_positioning_recorder` in
`parley_tests/glyph_positioning_recorder`, `publish = false`, with only the
Phase 2 parts filled in:

```
parley_tests/glyph_positioning_recorder/
  Cargo.toml        # deps: parley_glyph_positioning_cases, serde_json
  www/
    harness.html    # static, checked in
    harness.js      # static, checked in
  src/
    lib.rs
    harness.rs      # payload builder + staging + unit tests
                    # driver.rs, skp_json.rs, compare.rs: Phase 4
```

Also in Phase 2 (pulled forward from Phase 6, because the crate must build and
lint from the moment it exists):

- Add the crate to the workspace `members` in the root `Cargo.toml`. It needs no
  `workspace.dependencies` alias — nothing depends on it.
- Add `--exclude parley_glyph_positioning_recorder` to **both** `NO_WASM_PKGS`
  and `NO_ANDROID_PKGS` in `.github/workflows/ci.yml`. It is native-only once
  Phase 4 adds `fantoccini`/`tokio`.

`serde_json` is the first `serde` dependency in this workspace. That is
sanctioned: it is confined to this native-only, matrix-excluded crate, which
needs it for `skp_parser` output in Phase 4 regardless. Build the payload with
`serde_json::json!` — no derive, no `serde` dependency of its own.

## The payload contract

The browser side knows **nothing** about the `Case`/`Run` grammar. The Rust side
maps grammar to CSS; `harness.js` applies opaque strings. This is the load-
bearing decision of Phase 2: adding weight/italic/family in v2 must touch Rust
only, and it means the untyped, unlinted file holds no logic that can drift from
the grammar.

```jsonc
{
  "container": "width:312.5px",                      // opaque CSS declarations
  "runs": [
    { "text": "foo bar", "css": "font-family:Roboto;font-size:17.2341px;letter-spacing:0.5px;word-spacing:-0.25px" }
  ]
}
```

- One `<span>` per entry, in order, applied as `span.style.cssText = run.css`
  and `span.textContent = run.text`.
- Format floats with Rust's `{}` (shortest round-trip). Font sizes are never
  near a 1/100px boundary — Phase 1 samples them with ≥0.0005px of margin — so
  the decimal string cannot truncate to a different hundredth than the Parley
  side computes from the `f32`.
- `font-size` is set per run, never on the container: the container keeps
  `font-size: 0` to zero Blink's strut (Phase 1 decision).

## `harness.js` API

The page exposes exactly one global, `window.parleyHarness`, with two entry points
and a `run` helper that adapts them to WebDriver's `/execute/async` (which
supplies its callback as the final argument):

```js
parleyHarness.run(() => parleyHarness.initHarness(), arguments[0])

parleyHarness.run(
    () => parleyHarness.renderAndCapture(arguments[0], arguments[1]),
    arguments[2])
```

Both resolve to a result envelope and **never** leave a promise rejected:

```jsonc
{ "ok": true,  "width": 312.5, "height": 684.2 }
{ "ok": false, "error": "content 312.5x2210 exceeds viewport 1280x2048" }
```

A rejected promise in `/execute/async` surfaces to the driver as a *script
timeout*, not as an error message, so every path must `try`/`catch` and resolve
the callback with `ok: false`. The driver treats `ok: false` as fatal.

`initHarness()` calls `.load()` on every `FontFace` in `document.fonts`, awaits
them, and fails unless all report `status === "loaded"`. CSS-connected
`@font-face` rules load lazily, so an un-triggered face would otherwise sit at
`"unloaded"` forever and a silent fallback to a system font would render every
case in the wrong typeface. No family list is passed in — iterating
`document.fonts` covers exactly the staged faces.

`renderAndCapture(payload, skpDir)`:

1. Removes and recreates the container element (not just its children), applies
   `payload.container`, and appends one span per run.
2. Fails with `ok: false` if the content rect exceeds `innerWidth`/
   `innerHeight` — a clipped SKP would silently *drop* glyphs rather than
   mismatch them.
3. Awaits two `requestAnimationFrame` ticks, then calls
   `chrome.gpuBenchmarking.printToSkPicture(skpDir)`.

Build the DOM with `createElement`/`textContent` only. **Never `innerHTML`, and
never leave whitespace between spans** — under `white-space: pre-wrap` a stray
newline in the DOM is a rendered space, which would appear as a phantom glyph
Parley does not have.

## `harness.html`

Minimal by decision. Only the properties Phase 1 requires, plus a reset;
everything else is the pinned Chrome's UA default:

```css
html, body { margin: 0; padding: 0; }
#content {
  font-size: 0;                          /* zero Blink's strut  */
  white-space: pre-wrap;                 /* Parley hangs spaces */
  text-rendering: geometric-precision;
}
```

Do **not** set `font-variant-ligatures`. Parley already disables
`liga`/`clig`/`dlig`/`hlig` when letter-spacing is non-zero
(`parley/src/shape/mod.rs:88`), which is what Blink does; `nearly_zero` is
`< f32::EPSILON`, so the smallest sampled non-zero spacing (1/256) is
unambiguously non-zero on both sides. Pinning the property would *break* the
match, not tighten it.

One static `@font-face` per `FONTS` entry, `src: url("<family>.ttf")`. v1 has
exactly one. That filename↔family coupling is the one thing multi-font support
must generalise (generated CSS, or the `FontFace` array-buffer API as
`parley_tests/linebreaking_browser_recorder` uses).

## `harness.rs` API

Everything Phase 4's driver needs from Phase 2:

```rust
harness::VIEWPORT_WIDTH   // u32, set once per session
harness::VIEWPORT_HEIGHT  // u32, set once per session
harness::payload(&Case) -> serde_json::Value   // pass as the execute_async arg
harness::stage(&Path) -> std::io::Result<()>   // populate the bind mount
harness::container_css(&Case) -> String        // used by `payload`
harness::run_css(&Run) -> String               // used by `payload`
harness::font_file_name(&str) -> String
```

## Staging

`stage` populates the directory the container bind-mounts read-only:

```
<staged>/harness.html      # include_str!("../www/harness.html")
<staged>/harness.js        # include_str!("../www/harness.js")
<staged>/Roboto.ttf        # FONTS[0].bytes, filename from FONTS[0].family
```

Writing the font from `FONTS[i].bytes` rather than checking a copy into `www/`
is what makes byte-identity with the Parley side structural: a stale duplicate
is precisely the failure this harness cannot detect.

## What Phases 3/4 must preserve

- Chrome launch flags: `--allow-file-access-from-files` (the staged font is
  fetched over `file://`), `--hide-scrollbars`, `--force-device-scale-factor=1`,
  plus the parent plan's `--enable-gpu-benchmarking --no-sandbox
  --font-render-hinting=none`.
- Set the window to the fixed viewport **once** per session, from the constant
  in `harness.rs`. Never resize per case: the corpus tops out around 1000×700px,
  and a per-case resize adds a relayout-settling race to every capture. The
  constant lives in Rust only; JS reads `innerWidth`/`innerHeight`.
- Call `initHarness()` after navigating and before the first case.

## Verification

Deferred to Phase 4 by decision — Phase 2 ships nothing executable, and there is
no standalone demo hook. Two consequences to keep in mind during Phase 4 bring-
up, both of which present as position mismatches rather than named errors:

- **Every glyph off by the same constant** → the container is not at the
  document origin. Parley reports layout-relative x/y; Chrome reports
  document-absolute. The comparison assumes they coincide.
- **One run wrong, the rest correct** → a malformed declaration in that run's
  CSS string. The CSS parser drops malformed declarations silently, so a typo
  in a property name produces no error anywhere.

Phase 4 should also carry a deliberate **staleness test**: render case A,
capture, render case B, capture, assert the two captures differ. If
`printToSkPicture` records the last committed compositor frame rather than
forcing one, a reused session would lag by one case — which looks like
catastrophic parity failure, not a harness bug. If that test shows the double
`rAF` is unnecessary, it can be dropped.
