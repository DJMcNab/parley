# Chromium glyph-positioning parity harness

## Context

Parley needs a test harness that renders a small styled-text document in real,
sandboxed Chromium and compares the resulting **glyph positions** (not just
line-break points, which the existing `linebreaking_matches_chrome` harness
already covers) against Parley's own layout of the same document. This is a
much stronger fidelity signal than line-breaking parity: it exercises
per-glyph shaping/advance/kerning output directly, using Chromium's own
Skia-recorded draw commands as the oracle via
`chrome.gpuBenchmarking.printToSkPicture`.

Unlike the existing `linebreaking_browser_recorder` (which requires a human
to open real Chrome and click a clipboard button), this harness must be
**fully automated**: a pinned, network-isolated Chromium running in a
container, driven from Rust, so it can run continuously as a differential
fuzzer as well as gate CI against a small checked-in golden set.

This plan reflects an extended design discussion that resolved ~15 concrete
architecture decisions. It intentionally scopes v1 tightly (single bundled
font, no weight/italic, ASCII+BMP text only, flat non-overlapping style
runs) so the actual hard problem — glyph-level Chromium fidelity — isn't
obscured by a large style/grammar surface.

## Key resolved decisions (do not re-derive these)

- **Location**: three new crates under `parley_tests/`, following the exact
  existing convention (`parley_tests/linebreaking_cases` → crate
  `parley_linebreaking_cases`, directory name has the `parley_` prefix
  stripped, crate name has it restored). *(Phase 1 correction: the original
  two-crate split put the Parley-side extraction in the cases crate; it now
  lives in its own crate so the cases crate has no `parley` dependency. See
  [`…-phase1.md`](./glyph-positioning-chrome-parity-phase1.md).)*
- **Grammar**: one Rust struct, flat non-overlapping fully-styled runs, no
  per-property overlapping ranges, no nested tree. `FontFamily`/`FontWeight`/
  `FontStyle` omitted entirely from v1 (single bundled `Roboto-Regular.ttf`,
  no new font assets). Text is Unicode within the BMP (no surrogate pairs /
  4-byte-UTF-16 codepoints), sampled from characters present in the bundled
  font's own `cmap` (so shaping stays meaningful, no `.notdef` tofu). *(Phase 1
  corrections: `font_size` is per-**run**; `line_height` is **dropped** from the
  grammar entirely, fixed to CSS `normal`; the cmap sample excludes the `Cc`/
  `Cf`/`Co`/`Zs` hazard classes; U+0020 is inserted by a separate rule.)*
- **Browser driving**: `fantoccini` (WebDriver client) + a version-matched
  `chromedriver`, both pinned via Google's "Chrome for Testing" manifest
  (same version string as the pinned `chrome-headless-shell` build). One
  long-lived session/page; `execute()` mutates the DOM and calls
  `chrome.gpuBenchmarking.printToSkPicture(dir)` repeatedly — confirmed via
  the W3C spec that `/execute/sync` runs in the real main-world `window`
  context, so `window.chrome.gpuBenchmarking` is directly reachable. Launch
  flags required: `--enable-gpu-benchmarking --no-sandbox` (the latter is
  mandatory for the API to write files at all, not a security tradeoff we're
  choosing).
- **Container**: `docker` CLI (hardcoded — user has the docker-compat CLI
  locally; GitHub-hosted CI runners have real Docker; no podman/docker
  config knob), invoked via `std::process::Command` (no `bollard`/API
  client). Dockerfile pins Chromium + `chromedriver` from Chrome for Testing
  at image-*build* time; the running container has no route to the
  internet. The SKP output directory is a **bind-mounted volume** shared
  between host and container, so Rust reads `layer_*.skp` files directly —
  no `docker cp` round-trip. **Phase 0 correction:** Chrome for Testing
  publishes **no linux-arm64 build**, so the Chromium image must be
  `linux/amd64` — native on GitHub CI runners, emulated on an arm64 Mac.
  `skp_parser` has no such constraint and can be built for the host arch;
  SKPs parse fine across architectures. **Phase 3 corrections (both
  superseded by grilling before/during implementation — see
  `parley_tests/glyph_positioning_recorder/container/README.md`):**
  - `skp_parser` and Chromium/`chromedriver` were folded into **one combined
    multi-stage image**, not two separate ones — both stages target
    `linux/amd64` uniformly (matching CI's native arch was judged more
    valuable than avoiding local QEMU emulation for the Skia build stage).
  - **The running container does *not* have a route to the internet cut
    off after all.** The `--internal`-Docker-network design that would
    enforce this works on real Docker but was confirmed broken on Podman:
    Podman's `--internal` disables IP forwarding on the bridge interface
    entirely (not just the outbound NAT rule Docker skips), which also
    blocks the host-to-published-port path the WebDriver connection needs.
    Since this pipeline never runs in CI (only locally/manually — see Phase
    6), Podman's behavior is what actually matters. `chromedriver
    --allowed-ips` is the only access control now; accepted deliberately
    since inputs are self-generated test HTML, not adversarial.
  - **Phase 4 correction: the driver is attach-only.** It never runs
    `docker run` or `docker build` — the developer starts the container per
    `container/README.md` and the driver connects to it, configured entirely
    by environment variables. `skp_parser` is reached with `docker exec` into
    that named container. The documented run command also gains a **second
    bind mount** for the staged harness page, alongside the SKP mount.
- **SKP → glyph data**: `skia-safe` **cannot** intercept draw commands from
  pure Rust (`Canvas` has no Rust-side virtual dispatch — confirmed from the
  crate's own docs/README). Given the "Rust + browser JS only" language
  constraint, we build **Skia's own unmodified `skp_parser` tool** inside
  the container image (existing upstream C++, not code we author — a
  natural extension of the image-build-time network access already needed
  for Chromium), shell out to it, and parse its JSON draw-command dump in
  Rust. ~~We do our own **proper stack-based transform composition** from the
  raw `Save`/`Concat44`/`Restore` command stream (better than Canva's
  flat-first-`Concat44`-offset shortcut) — this is plain Rust logic, no C++
  needed for it.~~ **Phase 4 correction: there is no transform stack.** Phase 0
  found those commands do not appear at all — absolute coordinates are already
  baked into each `DrawTextBlob` origin — so Phase 4 takes that as the contract
  and hard-errors if any transform or save/restore command appears, rather than
  shipping a composition path that no captured data exercises. Implementing the
  stack then happens with a real example in hand. Must replicate Canva's
  discovered `serde_json`
  `arbitrary_precision`-vs-tagged-enum gotcha (read a `{type: String}` tag
  from `RawValue` bytes first, then re-deserialize into a concrete struct —
  do NOT use an internally-tagged enum derive directly on the command JSON).
- **Skia revision for `skp_parser`**: must match (not just "compatible
  with") the pinned Chromium build's bundled Skia, found via that Chromium
  tag's `DEPS` file (`src/third_party/skia` entry) on
  `chromium.googlesource.com`. This gives an *exact* match, which is
  actually a stronger guarantee than the versioned-compatibility-window
  concern that applied to the (rejected) `skia-safe` approach.
- **Rendering fidelity** (*corrected by the Phase 0 spike — the original
  decision named `text-rendering: geometric-precision` as the lever, and it
  is not*): launch Chromium with **`--font-render-hinting=none`**. Without
  it, headless Chrome defaults to `hinting: full` and quantizes every advance
  to a whole pixel, making exact parity impossible; `geometric-precision` was
  measured to have *no effect at all* on glyph positions. Keep it in the CSS
  anyway — harmless, and it may still matter on the real Win/Mac target. That
  target is cross-platform (Win/Mac) Chrome behavior; Linux headless
  Chrome-in-a-container is a stand-in oracle, and this flag is what makes the
  stand-in valid.
- **FreeType ascent/descent hack filter**: `hack_would_fire(hhea_descender,
  units_per_em, css_font_size)` (pure font-metrics math, no rendering) is
  used as a pre-filter **everywhere case generation happens** (fuzz loop
  and golden-corpus generation) — skip/reroll any `(font, size)` that would
  trigger it, rather than compensating for it in comparison logic. Blink's
  Linux/ChromeOS/Android/Fuchsia-only FreeType workaround
  (`FontMetrics::AscentDescentWithHacks` in
  `third_party/blink/renderer/platform/fonts/font_metrics.cc`) shifts
  ascent/descent by 1 unit when `use_subpixel_positioning` is set (which
  `text-rendering: geometric-precision` forces) and the font's descent, in
  physical pixels at the tested size, rounds down (fractional part in
  `(0, 0.5)`); it does not exist on Windows/macOS. **Phase 1 correction: no
  longer unverified — the formula is confirmed verbatim against
  `font_metrics.cc` at tag `151.0.7922.77`, and its precondition *holds*.**
  Phase 0 recorded the opposite because it read `subpixelText`'s absence as
  false; under `--font-render-hinting=none` the captures do set
  `subpixelText: true`, so the hack is live. For Roboto it fires for roughly
  **half** of the 10–30px range, which is the cost of the pre-filter.
- **Viewport**: *(**Phase 2 correction**: no longer auto-sized per case.)* A
  fixed oversized viewport, set **once** per session, with the harness failing
  loudly if content ever exceeds it. The corpus tops out around 1000×700px, so a
  per-case resize bought nothing and added a relayout-settling race to every
  capture, plus a per-case variable to a long-lived reused session.
- **Harness CSS** (*added in Phase 1; these bind Phase 2*):
  - **`white-space: pre-wrap`** on the container. Parley *hangs* overflowing
    whitespace — `line_break.rs:834-852` appends the space atom to the current
    line past `max_advance`, then breaks — which is exactly `pre-wrap`. The
    default `normal` collapses the space away entirely (so Blink paints no
    glyph where Parley has one), and `break-spaces` does **not** hang, so it
    would wrap where Parley hangs and diverge on every wrapped line. `pre-wrap`
    also preserves leading/trailing spaces, matching Parley, which is why the
    glyph corpus may contain them where the line-breaking corpus may not.
  - **`font-size: 0` on the block container**, so Blink's strut contributes
    ascent 0 / descent 0. Blink always adds a strut from the container's own
    font; Parley has none (`Extents::default()` is `over: 0, under: 0`, with a
    standing TODO to source initial extents from the primary font). Neutralising
    it is what makes Blink's line box the union over the spans only, i.e.
    Parley's model. Needs empirical confirmation that it does not trip a
    different Blink code path.
  - `text-rendering: geometric-precision` is kept, but note it is **not** the
    total no-op Phase 0 concluded: it feeds `subpixel_ascent_descent`, which
    skips ascent/descent rounding for fonts below ~3.2px. Outside v1's 10–30px
    range, but it matters if sizes ever go smaller.
- **`skp_parser` JSON gotchas** (*added in Phase 1; these bind Phase 4's
  deserializer*), all confirmed from Skia at the pinned revision:
  - Per-glyph position depends on the run's positioning mode.
    `kHorizontal` gives `x = blob.x + positions[i]`, `y = blob.y + coords[1]`,
    with `positions` a flat float array. `kFull` gives
    `x = blob.x + positions[i][0]`, `y = blob.y + positions[i][1]`, with
    `positions` an array of **2-element arrays** — a different JSON shape that
    the deserializer must handle. `kDefault` omits `positions` entirely and its
    positions are not recoverable.
  - `MakeJsonFont` uses `store_scalar`/`store_bool`, which **omit fields equal
    to their default**. `SkPaintDefaults_TextSize` is **12**, so a run at
    exactly 12px emits *no* `textSize` field — absence must default to `12.0`,
    not error. 12px is inside the sampled range.
  - `scaleX` and `skewX` are both written under
    `DEBUGCANVAS_ATTRIBUTE_TEXTSCALEX`, so a non-zero skew produces a duplicate
    JSON key. Harmless while v1 has no italic/synthetic oblique.
  - Runs may also carry optional `clusters` (glyph→text-offset) and `text`
    fields; absent in the Phase 0 captures, but free glyph-to-character mapping
    if Blink ever emits them.
- **Typeface identity**: the run side-table records the **PostScript name**
  (`name` ID 6), not a hash. `apply_font_typeface` registers
  `typeface->serialize()` as `data/N`, and `skp_parser FILE.skp data/N` dumps
  those bytes; a *validating* sfnt scan (reject candidates whose table
  directory doesn't parse or lacks `name`/`cmap` — a naive magic-byte scan of a
  real capture yields 9 false hits and 1 true one) then yields the name via
  `read-fonts`. This is stable across Skia bumps, unlike a hash of Skia's
  serialization wrapper, and directly comparable to the Parley side. If Chrome
  falls back to a *system* font, Skia may serialize by-descriptor with no
  embedded stream — the scan finding nothing is itself the fallback signal.
- **Comparison philosophy**: exact match modulo *explicitly modeled*
  Chromium quantization (font-size 1/100px truncation, 16.16 glyph-advance
  flooring). No fuzzy pixel-tolerance banding. **Phase 1 corrections:**
  - `skp_parser` emits at most **6 significant figures**, so literal exact
    match is unachievable.
  - Blink runs HarfBuzz at 16.16 scale, so *every* glyph advance is quantized
    to 1/65536px before accumulation — this is systemic, not incidental. Phase
    0's spot-check missed it only because at 24px/upem 2048 every advance is
    `units × 3/256`, exactly representable in 16.16.
  - The predicate is therefore
    `|parley_x − chrome_x| ≤ half_ulp_6sig(chrome_x) + i × 2⁻¹⁶`, with `i` the
    glyph's index within its line. A flat 1/64px tolerance was considered and
    **rejected**: it is ~9× looser than a 120-glyph line needs and exceeds one
    font unit at every size in the sampled range, so it would swallow exactly
    the kerning/advance/mark bugs this harness exists to find.
  - `parley_output` must accumulate x in **f64** rather than reuse
    `positioned_glyphs()`, whose f32 running offset would otherwise contribute
    a larger error than the drift being modelled.
  - The claim that 16.16 accumulation logic is "already reverse-engineered for
    `linebreaking_matches_chrome.rs`" is **false** — that test does not model
    16.16, it tolerates it via `RESIDUAL_SLACK_SUBPIXELS`. There is nothing to
    port.
  - Generated widths are capped so `|x| < ~1000px`.

  **Phase 4 corrections:**
  - The `i × 2⁻¹⁶` term and its `index_in_line` argument are **removed**. Phase
    4 models Blink's 16.16 quantization on the *Parley* side instead of
    tolerating it (see
    [`glyph-positioning-16-16-advances.md`](./glyph-positioning-16-16-advances.md)),
    so both sides accumulate identically and the predicate collapses to
    `|Δ| ≤ half_ulp_6sig(other)` on both axes — the serialisation floor alone.
    The 120-glyph and ~1000px caps stay, but their drift-bounding rationale no
    longer applies.
  - That also removes the need to know a glyph's line. Just as well: the
    obvious derivation — grouping by equal `y` — is **wrong**, because a
    combining mark carries a GPOS y-offset and so does not share its base's
    `y`, and the alphabet deliberately keeps 12 such marks.
  - **`compare` lives in `parley_glyph_positioning_cases`**, not the recorder:
    Phase 5's test needs the same comparison and must not depend on the
    recorder crate.
  - Pairing is emission order, index by index, with a sorted re-check on the
    failure path only, so a paint-order difference reports as such rather than
    as a wall of position diffs.
- **Golden CI set**: hand-written cases + promoted fuzz regressions + ~50
  deterministically-seeded generated cases. **Phase 1 correction (supersedes
  the original seed-only scheme):** *every* golden stores the **full `Case`**
  alongside the Chrome output, in one uniform format across all three
  categories; the seed is provenance only and is never regenerated from at test
  time. Seed-only saves ~13% (a `Case` is ~550 bytes against ~4 KB of output
  that must be committed regardless) while creating a sharp failure mode: any
  change to `from_seed` silently repoints every golden at a different case
  while the suite keeps passing. **Phase 4 correction:** handwritten cases have
  no separate source of truth — they are authored *as* golden files with an
  empty output section (`styles 0` / `fragments 0`) and filled in by
  `regenerate_goldens`, which re-records every existing file in place. That
  gives one uniform rule across all three directories, and promoted fuzz
  regressions already arrive in exactly that shape. The format gains an optional
  `note <text>` line so a handwritten case can say why it exists.
- **Serialization**: goldens use a **hand-rolled compact text format**, per the
  precedent in `linebreaking_matches_chrome.rs`. Neither Phase 1 crate takes a
  serde dependency; `serde_json` is confined to the native-only, matrix-excluded
  recorder, which needs it for `skp_parser` output anyway.
- **Normal CI never touches Docker**: the `#[test]` in `parley_tests`
  compares live Parley output against checked-in golden JSON only. Docker/
  Chromium/`chromedriver`/`skp_parser` are only exercised by (a) the golden-
  regeneration binary (developer-run, offline) and (b) the fuzz loop
  (manual/scheduled, not part of PR-gating CI).
- **Fuzzing model**: a long-running randomized differential-testing loop
  (not `cargo-fuzz`/libFuzzer — per-case Chromium round-trips are far too
  slow for coverage-guided fuzzing), reusing one browser session across
  many generated cases. On mismatch, writes the failing case to a scratch
  directory; a human reviews and manually promotes worthwhile ones into the
  checked-in regression corpus — mirrors the existing
  `PARLEY_TEST=accept` PNG-snapshot workflow already in `parley_tests`.
- **Delegation**: implement as much as possible via Sonnet subagents
  (parallel `Agent` calls per independent chunk below). If a Docker/
  permission wall is hit during implementation or verification, **stop and
  wait for user input — do not attempt to bypass** (no `sudo`, no disabling
  sandboxing, no faking results).

## Crate/file layout

```
parley_tests/
  glyph_positioning_cases/          # crate: parley_glyph_positioning_cases
    Cargo.toml                       # deps: rand, rand_chacha, read-fonts
    src/
      lib.rs                        # FONTS registry, Case, Run
      generate.rs                   # Case::from_seed, alphabet (OnceLock)
      chromium_quantization.rs      # font-size truncation, the 1/64 + 1/256 grids
      freetype_hack.rs              # hack_would_fire(...)
      glyph_output.rs               # schema + hand-rolled text reader/writer
      typeface.rs                   # validating sfnt scan, PostScript name
      compare.rs                    # Parley vs Chrome diff (Phase 4; moved
                                    # here from the recorder so the Phase 5
                                    # test can use it)
      harness_css.rs                # container_css/run_css (moved here from the
                                    # recorder so the report generator applies
                                    # byte-identical CSS)
  glyph_positioning_extract/         # crate: parley_glyph_positioning_extract
    Cargo.toml                       # deps: parley, fontique, the cases crate
    src/
      lib.rs                        # layout(&Case) -> Layout, parley_output()
  glyph_positioning_report/          # crate: parley_glyph_positioning_report
    Cargo.toml                       # deps: cases, extract, parley, glifo,
                                     # vello_cpu, icu_properties
    src/                             # developer tool, not used by CI; see
                                     # doc/glyph-positioning-report.md
  glyph_positioning_recorder/        # crate: parley_glyph_positioning_recorder
    Cargo.toml                       # native-only; NOT a workspace member of
                                      # the wasm/android CI matrix (see below)
    container/
      Dockerfile                     # pins chrome-headless-shell + chromedriver
                                      # (Chrome for Testing manifest) + builds
                                      # skia/tools/skp_parser from the DEPS-
                                      # matched Skia revision
    www/
      harness.html                   # static, loaded via file://
      harness.js                     # JSDoc-typed, no build step
    src/
      driver.rs                      # docker lifecycle, fantoccini session,
                                      # bind-mounted scratch dir
      skp_json.rs                    # shells out to skp_parser, parses its
                                      # JSON (RawValue tag-dispatch pattern);
                                      # no transform stack — see Phase 4
      bin/
        regenerate_goldens.rs        # (re)writes parley_tests/tests/glyph_positioning/golden/*
        fuzz_loop.rs                 # long-running differential loop
  tests/
    glyph_positioning/               # one uniform format; every file holds the
      handwritten/*.txt              # FULL Case + golden Chrome output, with the
      regressions/*.txt              # seed recorded as provenance only
      generated/*.txt                # ~50 deterministically-seeded cases
    glyph_positioning.rs             # the actual #[test]; depends only on
                                      # parley_glyph_positioning_{cases,extract}
                                      # (+ parley_dev) — NOT on the recorder crate
    mod.rs                           # add `mod glyph_positioning;`
```

## Implementation phases

### Phase 0 — feasibility spike (do this first, before building anything else)

Validate the riskiest unverified assumptions before investing in the full
pipeline:

1. Pick a specific Chrome-for-Testing version. Look up its `DEPS` file on
   `chromium.googlesource.com` for the exact `src/third_party/skia` git
   revision.
2. In a throwaway container, build `skia/tools/skp_parser` (GN+Ninja, Skia's
   native build — confirm this is a normal buildable target without
   needing Canva's Bazel-specific exposure patch, which was Bazel-only
   plumbing, not a change to `skp_parser.cpp` itself).
3. Launch pinned `chrome-headless-shell` with `--enable-gpu-benchmarking
   --no-sandbox`, drive it via `fantoccini`, build one trivial styled `<div>`
   with `text-rendering: geometric-precision`, call `printToSkPicture` into
   a bind-mounted directory, confirm `layer_*.skp` files appear on the host.
4. Run `skp_parser` against the captured `.skp`, confirm it produces a JSON
   command dump containing `DrawTextBlob`/`Concat44` entries with glyph
   IDs and positions.

If any step hits a hard wall (e.g. `skp_parser` isn't cleanly buildable
outside Canva's Bazel patching), stop and report back before proceeding —
this would mean revisiting the C++-shim alternative.

**This phase involves live `docker build`/`docker run` — if sandboxing
blocks these, stop and wait for user input rather than working around it.**

#### Phase 0 — DONE, all four steps passed

**Do not re-run this phase.** Results, evidence and build timings are in
[`glyph-positioning-chrome-parity-phase0.md`](./glyph-positioning-chrome-parity-phase0.md);
read it only if you need the *why* behind a correction. What later phases must
act on:

- Pins: Chrome for Testing **151.0.7922.77**, Skia
  **`f4eee5c6735d33a829ab2cfbf3fe23d0376e7992`**.
- `skp_parser` is a normal GN `test_app` — **no Bazel patching needed** — but
  needs **clang-19** (`debian:trixie-slim`); set `skia_enable_pdf=false`. Cold
  build ~4m46s; the naive single-stage image is 9.9 GB, so Phase 3 should use
  a multi-stage build.
- `printToSkPicture` is reachable from WebDriver's `/execute/sync` context and
  writes SKPs to the bind mount, as assumed.
- Two corrections, folded into the decisions above: the
  **`--font-render-hinting=none` launch flag** (not `geometric-precision`) is
  what gates linear metrics, and the Chromium image must be **`linux/amd64`**.
- `Save`/`Concat44`/`Restore` did **not** appear at all in a simple document —
  absolute coordinates were already baked into each `DrawTextBlob` origin.
  Write the transform stack defensively, but do not *assume* it is the source
  of positions.

Working artifacts — both Dockerfiles, the harness page, the `fantoccini`
driver and the captured SKP/JSON — are in **`chrome_testing_spike/`** at the
repo root (gitignored via an internal `*`; see its `README.md`). Phases 3 and 4
should crib from these rather than rebuilding the pipeline from this text.

### Phase 1 — case generation and Parley-side extraction

**Fully specified in
[`glyph-positioning-chrome-parity-phase1.md`](./glyph-positioning-chrome-parity-phase1.md)
— follow that file, not this summary.** It supersedes several bullets above,
each annotated inline with a "Phase 1 correction".

In brief: two crates (not one). `parley_glyph_positioning_cases` holds the
`FONTS` registry, `Case`/`Run` and `Case::from_seed`, the per-font sampling
alphabet, the Chromium quantization helpers, `hack_would_fire`, the golden
schema with its hand-rolled text reader/writer, and the sfnt/PostScript-name
helper — deps `rand`, `rand_chacha`, `read-fonts` only.
`parley_glyph_positioning_extract` (provisional name) owns `layout()` and
`parley_output()` and is the *only* place the Parley side is constructed, so
the Phase 5 test and the Phase 4 fuzz loop cannot drift apart.

The output schema is a deduplicated `styles` table plus a flat
`Vec<{ id, x, y, style }>` with **no line concept** — Chrome exposes a true
per-glyph `y`, so lines need not be inferred, and the schema survives future
vertical-align work. *(Superseded by bring-up step B13 below: the glyph list is now
grouped into fragments, each with an origin the glyph positions are relative to. The
"no line concept" half still holds.)*

### Phase 2 — browser harness (delegate to a Sonnet subagent)

**Fully specified in
[`glyph-positioning-chrome-parity-phase2.md`](./glyph-positioning-chrome-parity-phase2.md)
— follow that file, not this summary.** It supersedes several bullets above,
each annotated inline with a "Phase 2 correction".

In brief: `www/harness.html` + `www/harness.js` (no build step, loaded via
`file://`), plus the Rust half that feeds them, in a new
`parley_glyph_positioning_recorder` crate whose Phase 4 files are left for
Phase 4. The page is a *dumb renderer*: the `Case`/`Run` → CSS mapping lives in
Rust and the browser applies opaque CSS strings, so the grammar exists in one
typed language only. Two entry points, `initHarness()` and
`renderAndCapture(payload, skpDir)`, both driven through `/execute/async`.

Phase 2 also does this crate's share of the Phase 6 workspace/CI wiring, since
the crate must build and lint from the moment it exists.

#### Phase 2 — DONE (implemented, never executed)

The crate, page, script, payload builder and staging function all exist and are
unit-tested; the workspace member and the two `ci.yml` excludes are already
added, so Phase 6 no longer needs to do that part. Nothing has run in a browser
yet — Phase 4 is the first execution of any of it.

### Phase 3 — container (delegate to a Sonnet subagent, verify manually)

**Start from `chrome_testing_spike/Dockerfile.chrome` and `Dockerfile.skia`**
(gitignored spike folder at the repo root) — they already build and run. Fold
them into one image, add a multi-stage build so the Skia checkout is dropped,
and fix the two shortcuts its `README.md` flags (`--allowed-ips=`, no network
isolation).

`container/Dockerfile`:
- Minimal Debian/Ubuntu base + apt-installed shared libs `chrome-headless-shell` needs.
- Pin exact Chrome-for-Testing version; download `chrome-headless-shell` +
  matched `chromedriver` with checksum verification where available.
- Fetch the DEPS-matched Skia revision, build `skp_parser` only (not full
  Skia/Chromium).
- Document the exact version-pinning procedure (where the version string
  and Skia revision came from) in a comment or adjacent README, since this
  is exactly the kind of pin that needs a documented bump procedure.

Manual verification step (not delegated — needs real `docker build`/`run`):
confirm the image builds and Phase 0's spike still passes against the final
image.

### Phase 4 — recorder crate (sequenced after Phase 0/3)

**Fully specified in
[`glyph-positioning-chrome-parity-phase4.md`](./glyph-positioning-chrome-parity-phase4.md)
— follow that file, not this summary.** It supersedes several bullets above,
each annotated inline with a "Phase 4 correction".

In brief: `driver.rs` (attach-only container, one long-lived session),
`skp_json.rs` (`docker exec skp_parser`, `RawValue` tag dispatch, no transform
stack), `compare` in the *cases* crate, and the `regenerate_goldens` /
`fuzz_loop` binaries. `chrome_testing_spike/driver/src/main.rs` is a working
reference for the capabilities blob, launch flags and capture call, and
`chrome_testing_spike/*.json` are real `skp_parser` outputs.

Phase 4 is the **first execution of anything** in this pipeline, so its largest
section is a gated bring-up checklist resolving the questions Phases 1 and 2
left open — each with a probe, an expected answer and a pre-agreed contingency.
Nothing else runs until it is green.

Phase 4 also carries a spike that reaches outside the harness: modelling
Blink's 16.16 advance quantization inside `parley_engine`, unconditionally, to
measure its blast radius. That has its own document,
[`glyph-positioning-16-16-advances.md`](./glyph-positioning-16-16-advances.md),
and is what allows the comparison predicate to drop its accumulation term.

### Phase 5 — CI-facing test + initial golden dataset

#### Phase 5 — DONE

**Fully recorded in
[`glyph-positioning-chrome-parity-phase5.md`](./glyph-positioning-chrome-parity-phase5.md)
— read it for what actually happened**, including a significant correction to
Phase 4's findings: the decomposition-cluster bug (found there via the letter/
word-spacing workaround) turned out to fire on ~86% of generated cases, not the
one anecdotal case Phase 4's smaller fuzzing sample suggested. That reshaped the
initial corpus.

In brief: `parley_tests/tests/glyph_positioning.rs` is a single `#[test]`,
registered in `tests/mod.rs`, with no container/network/Docker dependency. It
walks four directories under `tests/tests/glyph_positioning/` —
`handwritten/`, `regressions/`, `generated/` (expected to pass) and a new
**`known_failing/`** (checked-in repros of real, tracked bugs, asserted to
*keep failing* so a fix shows up as a test failure telling you to promote the
case out). B8 is landed. Given the decomposition bug's real prevalence, the
initial corpus is a curated **15 passing + 15 known-failing** cases rather
than the originally planned 50, and `regenerate_goldens` no longer
auto-generates new `generated/` seeds from a blind range (unsound at this
failure rate) — it only re-records whatever is already on disk.

### Bring-up step B13 — fragment-origin snapping

#### DONE

**Fully recorded in
[`glyph-positioning-fragment-snapping.md`](./glyph-positioning-fragment-snapping.md).**
The last open bring-up step, and the reason `MAX_RUNS` was pinned to 1. Blink places
each fragment on a line at the previous one's width ceil-rounded onto `LayoutUnit`'s
1/64 px grid; that is now modelled in
`parley_glyph_positioning_extract::parley_output` rather than tolerated, so the
comparison stays exact. `MAX_RUNS` is 4 again.

Three things in the decisions above are superseded by it:

- **The output schema is fragment-structured, not flat.** `GlyphOutput` is a list of
  fragments carrying an origin plus glyph offsets *local* to it, with
  `GlyphOutput::glyphs()` deriving the flat absolute view. Both the snapping and the
  serialisation floor need the origin and the offset kept apart. There is still no
  line concept.
- **The comparison predicate is
  `|Δ| ≤ half_ulp_6sig(origin) + half_ulp_6sig(offset)`.** `skp_parser` rounds those
  two numbers independently, so the floor on their sum is the sum of their floors.
  Phase 4's "the predicate collapses to `half_ulp_6sig(other)`" held only while a
  line could hold one fragment, where the origin is always 0.
- **`compare` has a third failure mode, `Fragmentation`**, checked between glyph
  count and positions.

### Phase 6 — workspace/CI wiring

- ~~Add all three new crates to the root `Cargo.toml` workspace `members` (and
  `workspace.dependencies` aliases for `parley_glyph_positioning_cases` and
  `parley_glyph_positioning_extract`).~~ **Already done** by Phases 1 and 2.
- ~~Exclude `parley_glyph_positioning_recorder` from the wasm/android CI
  matrix jobs.~~ **Already done** by Phase 2: it is in both `NO_WASM_PKGS` and
  `NO_ANDROID_PKGS` in `ci.yml`, alongside `xtask`/`parley_bench`/
  `parley_data_gen`.
- `parley_glyph_positioning_cases` and `…_extract` need no exclusion (no
  heavy/native-only deps; `parley` is already in the matrix), same treatment as
  `parley_linebreaking_cases`.
- No new CI job needed for Docker/Chromium — the new `#[test]` runs as part
  of the existing `cargo nextest run --workspace` step, using only the
  checked-in golden data.

## Verification

- `cargo test -p parley_tests glyph_positioning` passes using only checked-in
  golden data, no network/docker access required, on a clean checkout.
- `cargo clippy --workspace --all-features` (native) is clean; wasm/android
  matrix jobs still succeed with the recorder crate excluded.
- Manually run `cargo run -p parley_glyph_positioning_recorder --bin
  regenerate_goldens` end-to-end and confirm it reproduces byte-identical
  golden files (idempotency check).
- Manually run the fuzz loop briefly and confirm it (a) finds zero
  mismatches on a short run against current `main`, or (b) any mismatches
  found are genuine and get written to the scratch directory in the
  expected format.
