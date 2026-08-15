# Phase 0 spike results — Chromium glyph-positioning parity

Evidence log for Phase 0 of [`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md).
Ran 2026-08-12; **all four steps passed**, no hard wall.

Read this if you need the *why* behind a Phase 0 correction, a build timing,
or the exact shape of `skp_parser`'s output. The plan itself carries the
decisions you must act on — you do not need this file to implement Phases 1–6.

Working artifacts are in **`chrome_testing_spike/`** at the repo root
(gitignored via an internal `.gitignore` containing `*`, so it never appears
in `git status`). Its `README.md` has the rebuild/run commands. Reuse those
Dockerfiles and the fantoccini driver for Phases 3–4 rather than starting over.

## Pins established

- Chrome for Testing **Stable 151.0.7922.77**
- Its Chromium `DEPS` gives
  `'skia_revision': 'f4eee5c6735d33a829ab2cfbf3fe23d0376e7992'`

## Step 2 — `skp_parser` buildability

It is a plain `test_app("skp_parser")` target in Skia's **top-level
`BUILD.gn`** (sources `tools/skp_parser.cpp`, deps `:skia` + `:tool_utils`,
gated on `skia_use_libpng_decode`). Normal `git-sync-deps` + `fetch-gn` +
`gn gen` + `ninja` — **no Bazel patching needed**, confirming the plan's
assumption. Two gotchas:

- Debian **bookworm's clang-14 cannot compile the C++20 `<ranges>` usage** in
  `src/pdf/SkPDFTag.cpp` against libstdc++-12. Use `debian:trixie-slim`
  (clang-19). A toolchain problem, not a `skp_parser` problem.
- `skia_enable_pdf=false` avoids that translation unit entirely and cuts build
  time; `skp_parser` does not need PDF.

Build timing (8 cores, native `linux/arm64`, cold cache, **286 s ≈ 4m46s**):

| stage | seconds |
| --- | --- |
| base image + apt (git/python3/clang-19/ninja) | 21 |
| shallow `git fetch` of the pinned Skia rev | 9 |
| `tools/git-sync-deps` | 109 |
| `bin/fetch-gn` + `gn gen` | 10 |
| `ninja skp_parser` (full Skia + tool_utils) | **134** |

Rebuilds with a warm layer cache skip everything but the last row. The naive
single-stage image is **9.9 GB** (whole checkout + build tree) — Phase 3 should
use a multi-stage build copying only the `skp_parser` binary. The Chromium
image is 828 MB.

## Steps 3 and 4 — capture and parse

- `chrome.gpuBenchmarking.printToSkPicture` **is** reachable from WebDriver's
  `/execute/sync` main-world context, as assumed.
- `layer_0.skp` appears on the host through the bind mount (172 KB, magic
  `skiapict` v109), with the `@font-face` Roboto **embedded** in the SKP —
  confirming the webfont was really used and not a system fallback.
- `skp_parser FILE.skp` prints pretty JSON (`{version, commands}`) built from
  `DebugCanvas::toJSON`, containing `DrawTextBlob` entries with per-run
  `glyphs`, `positions`, blob origin `x`/`y`, and `font.textSize`.

## Finding 1 — `geometric-precision` is a no-op; the launch flag is not

The A/B matrix over `text-rendering: geometric-precision` × `--font-render-hinting`
(four combinations, captured in `chrome_testing_spike/ab_gp*_hint*.json`): the
CSS property changed *nothing*, the flag changed *everything*.

```
hinting=full (default):  pos=[0, 17,      30,      36,      42,      56]
hinting=none:            pos=[0, 17.1094, 29.8242, 35.6484, 41.4727, 55.1602]
```

Without `--font-render-hinting=none`, headless Chrome quantizes every advance
to a whole pixel and exact parity is impossible.

## Finding 2 — no transform commands appeared at all

A simple document produced only `DrawPaint`, 2× `DrawRect`, 4×
`DrawTextBlob` — no `Save`/`Concat44`/`Restore` — with absolute page
coordinates already baked into each blob's origin `x`/`y`. The stack-based
transform composition is still worth writing defensively (device scale factor,
composited layers), but it is not exercised by basic cases and must not be
*assumed* to be the source of positions.

## Fidelity spot-check

24 px "Hello AVWA world" against the raw Roboto `hmtx` (upem 2048), hinting
off: Chrome's cumulative positions match linear advances **exactly** to printed
precision. The only deviations are −87 and −43 font units at "AV" and "WA" —
genuine GPOS kerning. Chrome emits true linear advances plus real shaping, so
the exact-match comparison philosophy looks well-founded.

## Still unverified

~~The FreeType ascent/descent hack formula. The captured fonts report
`subpixel` as unset (false) even with hinting disabled, and the hack is
documented as firing only when `use_subpixel_positioning` is set — so its
trigger condition needs re-checking against source before Phase 1 ports
`hack_would_fire`.~~ **Resolved during Phase 1 planning — and this paragraph was
wrong. See "Corrections" below.**

## Corrections (made while planning Phase 1)

Two claims above did not survive re-checking against the raw artefacts and
Chromium/Skia source. Both are fixed in the parent plan; recorded here so this
file is not read as still true.

### The hack's precondition *does* hold

`subpixelText` was read as false because it is *absent* from the JSON. But
`MakeJsonFont` uses `store_bool(…, false)`, which **omits any field equal to
its default** — absence means false, and presence means true. Re-reading the
captures:

| capture | `hinting` | `subpixelText` |
| --- | --- | --- |
| `ab_gp*_hintdefault.json`, `layer_0.json` | `full` | *absent* (false) |
| `ab_gp*_hintnone.json`, `layer_0_nohint.json` | `none` | **`true`** |

So under `--font-render-hinting=none` — the configuration we chose —
`use_subpixel_positioning` **is** set and the hack is live. The formula itself
is confirmed verbatim from `font_metrics.cc` at tag `151.0.7922.77`:
`descent < SkScalarToFloat(metrics.fDescent) && ascent >= 1` after
`SkScalarRoundToScalar`, i.e. descent's fractional part in `(0, 0.5)` — exactly
as the parent plan stated. For Roboto it fires for roughly half of the 10–30px
range.

The `y = 22` baseline in these captures is consistent: `hhea` ascent
`1900 × 24 / 2048 = 22.2656`, rounded to 22, with descent `5.8594` whose
fractional part is outside `(0, 0.5)` — so the hack correctly did *not* fire in
this particular sample. That also confirms Blink reads **`hhea`**, not `OS/2`
(`win` would give 23, `typo` 18).

### `geometric-precision` is not a *total* no-op

Finding 1 is right that it changes nothing about glyph x-positions. But it
feeds `subpixel_ascent_descent`, which skips ascent/descent rounding entirely
for very small fonts (`fAscent < 3`, i.e. below roughly 3.2px for Roboto).
Outside the 10–30px range v1 samples, so the practical conclusion stands — keep
the CSS property, and `--font-render-hinting=none` remains the real lever — but
the property does have a code path.

### Also learned from these artefacts

- The full sfnt is embedded in `skp/layer_0.skp` at byte offset 240, and its
  `name` ID 6 reads `Roboto-Regular`. This is how Phase 4 identifies typefaces.
  A naive magic-byte scan yields **9 false candidates and 1 true one**, so the
  scan must validate the table directory.
- `skp_parser` emits at most **6 significant figures** across every captured
  file (widest value `-17.6836`), which is why the parent plan's exact-match
  comparison had to be replaced with a half-ulp predicate.

## Spike-only shortcuts not to copy

- The spike's `chromedriver` runs with `--allowed-ips=`, which that build logs
  as "All remote connections are allowed"; it was safe only because the port
  was published to `127.0.0.1`. Phase 3 must tighten this.
- The spike container was **not** network-isolated. The real image must be.
