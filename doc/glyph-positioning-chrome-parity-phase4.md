# Phase 4 — recorder crate

Implementation spec for Phase 4 of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md).
Read the parent plan first, then
[`…-phase2.md`](./glyph-positioning-chrome-parity-phase2.md) for the harness
contract this codes against. This file is the authority on what Phase 4 builds.

Phase 4 is the **first execution of anything** in this pipeline. Phases 1 and 2
shipped unit-tested code that has never met a browser, and Phase 3 shipped an
image that has only been smoke-tested by hand. Accordingly the largest section
here is not the code but the [bring-up checklist](#bring-up-gate): a set of
questions Phases 1 and 2 explicitly left unresolved, each with the probe that
answers it and what to do if the answer is the unexpected one. **No golden or
fuzz binary runs until that checklist is green.**

Phase 4 also carries a spike that reaches outside the harness: modelling
Blink's 16.16 advance quantization inside `parley_engine`. That has its own
document, [`glyph-positioning-16-16-advances.md`](./glyph-positioning-16-16-advances.md),
and is sequenced after the recorder can capture.

## Deliverables

```
parley_tests/glyph_positioning_cases/
  src/compare.rs                   # NEW — moved here from the recorder
  src/glyph_output.rs              # amended: optional `note` line
  src/chromium_quantization.rs     # amended: x_matches loses its index argument
parley_tests/glyph_positioning_recorder/
  Cargo.toml                       # + fantoccini, tokio
  container/README.md              # amended: second bind mount
  src/
    driver.rs                      # container attach, session, capture
    skp_json.rs                    # skp_parser invocation + deserializer
    bin/
      regenerate_goldens.rs
      fuzz_loop.rs
  tests/fixtures/*.json            # real captures, checked in during bring-up
```

Phase 4 does **not** produce the ~50-case golden corpus or the CI test; those
stay in Phase 5, so the corpus is recorded once, against a Parley that already
quantizes.

## Corrections to earlier phases

Each of these supersedes something in the parent plan or a phase doc, and is
annotated there as a "Phase 4 correction".

1. **No transform stack.** The parent plan called for "proper stack-based
   transform composition" from `Save`/`Concat44`/`Restore`. Phase 0 found those
   commands do not appear at all — absolute coordinates are baked into each
   `DrawTextBlob` origin. Phase 4 takes that as the contract and **hard-errors
   if any transform or save/restore command appears**, rather than writing a
   composition path that no captured data exercises. Implementing the stack is
   then a change made with a real example in hand.
2. **`compare` lives in the cases crate**, not the recorder. Phase 5's test
   needs the same comparison and must not depend on the recorder; duplicating
   it is the drift Phase 1 restructured to avoid.
3. **Attach-only container.** The driver never runs `docker run` or
   `docker build`. See [Environment](#environment).
4. **The predicate loses its accumulation term.** With 16.16 modelled on the
   Parley side, `x_matches` becomes `|Δ| ≤ half_ulp_6sig(chrome_x)` — the same
   form as `y_matches` — and `index_in_line` / `ADVANCE_DRIFT_PER_GLYPH`
   disappear. See [Comparison](#comparison).
5. **Handwritten cases are golden files with an empty output section**, not a
   Rust list. The format gains an optional `note` line.

## Environment

> **Superseded.** The transport described in this section — `docker run` with two
> bind mounts, 6 env vars, `docker exec` for `skp_parser` and cleanup — was replaced by
> an in-container HTTP agent. See
> [`glyph-positioning-recorder-agent.md`](./glyph-positioning-recorder-agent.md) for
> what it looks like today; `container/run.sh` now starts the container, so nothing
> here is a command to actually run any more. The rest of this section is kept for
> history — the bring-up findings below (B1–B13, the strut fix) are unaffected by the
> transport and remain the authority on those questions.

The container is started by the developer, per
`parley_tests/glyph_positioning_recorder/container/README.md`. The driver only
attaches. That README's `docker run` needs a **second bind mount** for the
staged harness page, which Phase 4 adds:

```sh
docker run -d --name parley-recorder \
  -e CHROMEDRIVER_ALLOWED_IP=192.168.127.1 \
  -p 127.0.0.1:9515:9515 \
  -v "$PWD/skp:/skp" \
  -v "$PWD/www:/harness:ro" \
  parley-glyph-recorder:latest
```

Configuration is entirely by environment variable — there is no config file and
no CLI plumbing for these:

| variable | default | meaning |
| --- | --- | --- |
| `PARLEY_GLYPH_WEBDRIVER` | `http://127.0.0.1:9515` | chromedriver endpoint |
| `PARLEY_GLYPH_CONTAINER` | `parley-recorder` | `docker exec` target |
| `PARLEY_GLYPH_SKP_HOST_DIR` | *required* | host side of the SKP mount |
| `PARLEY_GLYPH_SKP_DIR` | `/skp` | container side of the same |
| `PARLEY_GLYPH_WWW_HOST_DIR` | *required* | host side of the harness mount |
| `PARLEY_GLYPH_WWW_DIR` | `/harness` | container side of the same |

There is deliberately **no startup validation** that the two host/container
pairs actually describe the same mounts; a mismatch surfaces as the first
capture failing to find its SKP.

`docker` is invoked as a hardcoded command name via `std::process::Command`,
per the parent plan (the user's docker-compat CLI is Podman; nothing here may
branch on the engine).

## `src/driver.rs`

Session setup, once:

1. `harness::stage(PARLEY_GLYPH_WWW_HOST_DIR)` — writes `harness.html`,
   `harness.js` and `Roboto.ttf`.
2. Connect `fantoccini` to `PARLEY_GLYPH_WEBDRIVER` with the capabilities blob
   from `chrome_testing_spike/driver/src/main.rs`, binary
   `/opt/chrome-headless-shell-linux64/chrome-headless-shell`, args:
   `--enable-gpu-benchmarking --no-sandbox --font-render-hinting=none
   --allow-file-access-from-files --hide-scrollbars
   --force-device-scale-factor=1 --disable-dev-shm-usage`.
   The first three are load-bearing (parent plan); the rest are Phase 2's list.
3. `set_window_size(harness::VIEWPORT_WIDTH, harness::VIEWPORT_HEIGHT)` —
   **once**, never per case.
4. Navigate to `file://{PARLEY_GLYPH_WWW_DIR}/harness.html`.
5. Set the script timeout explicitly (the WebDriver default of 30s applies to
   `/execute/async`; capture should be far under it, so a low value — 10s —
   turns a hang into a prompt error).
6. `parleyHarness.run(() => parleyHarness.initHarness(), arguments[0])`.

Per case:

1. Delete `layer_*.skp` from the SKP directory. **Bring-up correction: this must
   be done inside the container (`docker exec`), not on the host side of the
   mount** — see [B5](#b5-never-delete-the-skp-from-the-host).
2. One `/execute/async` call to `renderAndCapture(payload, PARLEY_GLYPH_SKP_DIR)`
   with `harness::payload(&case)`. `ok: false` in the envelope is fatal and its
   `error` is propagated verbatim.
3. Assert **exactly one** `layer_*.skp` now exists. Zero is a capture failure;
   two or more is fatal — `printToSkPicture` records no per-layer transform, so
   multiple layers cannot be composed back into document space.
4. No polling and no size-stability check: `printToSkPicture` is a binding on
   the renderer main thread and the `/execute/async` callback resolves on that
   same thread, so the write is complete when the driver hears back. Bring-up
   step [B5](#bring-up-gate) is what actually establishes this.

Session recycling is on error only — see [`fuzz_loop`](#srcbinfuzz_looprs).

## `src/skp_json.rs`

> **Superseded.** `docker exec` is gone; `skp_parser`'s JSON now arrives over HTTP via
> `crate::agent::AgentClient` (see
> [`glyph-positioning-recorder-agent.md`](./glyph-positioning-recorder-agent.md)), and
> this module is pure parsing — no `std::process::Command` left in it. The
> deserialization approach described below (tag-first `RawValue` reads, the command
> allow/deny lists, the positioning-mode dispatch) is unchanged by that move.

`docker exec {container} skp_parser {container_path}` → JSON on stdout →
`GlyphOutput`.

**Deserialization.** Read each element of `commands` as a `&RawValue`, pull a
`{ "command": String }` tag out of it, then re-deserialize that same `RawValue`
into the concrete struct for that tag. Do **not** derive an internally-tagged
enum: that buffers through `serde_json::Value` and is the documented
`arbitrary_precision` hazard the parent plan requires avoiding. The tag-first
pattern sidesteps the feature interaction entirely rather than depending on a
feature not being enabled somewhere in the graph.

**Command handling.** Three sets, and every command falls in exactly one:

- *Consumed*: `DrawTextBlob`.
- *Ignored*: `DrawPaint`, `DrawRect`, and the clip commands. Start the list at
  what Phase 0 actually captured (`DrawPaint`, `DrawRect`) and extend it as
  bring-up step [B6](#bring-up-gate) reports more.
- *Fatal*: everything else, including every `Save`/`SaveLayer`/`Restore`/
  `Concat44`/`SetMatrix`/`Translate`/`Scale` — an unmodelled transform produces
  a uniform offset, which Phase 2's doc already flags as the hardest failure to
  diagnose, so it must never be silently skipped.

A `DrawTextBlob` with `visible: false` is fatal rather than skipped; we do not
know what would produce one.

**Per run.** Each `DrawTextBlob` has `x`, `y` and a `runs` array. Per run:

- `font.textSize`, **defaulting to `12.0` when absent** — `MakeJsonFont` omits
  fields equal to their default and `SkPaintDefaults_TextSize` is 12, which is
  inside the sampled size range.
- `font.typeface.data`, a `"data/N"` key.
- `positions`, whose shape selects the positioning mode:
  - a flat float array → `kHorizontal`:
    `x = blob.x + positions[i]`, `y = blob.y + coords[1]`.
  - an array of 2-element arrays → `kFull`:
    `x = blob.x + positions[i][0]`, `y = blob.y + positions[i][1]`.
  - absent → `kDefault`, whose positions are unrecoverable: fatal.
- `glyphs`, parallel to `positions`.

Numbers are parsed as `f64` and narrowed to `f32`; `skp_parser` emits at most 6
significant figures, so nothing is lost.

**Typeface identity.** Every capture asserts the runs reference exactly one
distinct `data/N` key — two or more means a font fallback occurred. The bytes
behind that key are fetched **once per session** with a second invocation,
`skp_parser {file} {key}`, run through
`parley_glyph_positioning_cases::scan_for_valid_sfnts` (which must find exactly
one valid sfnt) and then `postscript_name_from_bytes`. The resulting name is
reused for every style in the session. Per-capture re-extraction would double
the `docker exec` cost of every fuzz iteration to re-derive something that
cannot change within a session; the failure it would additionally catch —
whole-document fallback — is already caught by `initHarness` asserting the
`@font-face` loaded.

`scaleX`/`skewX` collide under one JSON key upstream. v1 has no italic or
synthetic oblique, so the key is read as `scaleX` and a duplicate is ignored by
`serde_json`'s last-wins behaviour; this is recorded, not handled.

## Comparison

`parley_glyph_positioning_cases::compare`:

```rust
pub fn compare(parley: &GlyphOutput, chrome: &GlyphOutput) -> Result<(), Mismatch>
```

**Predicate.** `x_matches(parley_x, chrome_x)` and `y_matches(parley_y,
chrome_y)` are both `|Δ| ≤ half_ulp_6sig(other)`. The `i × 2⁻¹⁶` accumulation
term and its `index_in_line` argument are **removed**: once Parley quantizes
advances to 1/65536 (see the 16.16 doc) and `parley_output` sums them in f64 —
exact, since 1000 × 65536 is far inside f64's mantissa — the two sides
accumulate identically and only `skp_parser`'s 6-significant-figure
serialisation separates them.

This also removes the need to know which line a glyph is on. That is a relief
rather than a simplification: the obvious derivation, grouping glyphs by equal
`y`, is **wrong** — a combining mark carries a GPOS y-offset, so it does not
share its base's `y`, and Phase 1 deliberately keeps 10 `Mn` and 2 `Me` marks
in the alphabet. Parley's line structure was the only sound source, and
plumbing it through would have meant a wrapper type around `GlyphOutput`.

Phase 1's 120-glyph and ~1000px caps stay, but their stated rationale —
bounding accumulated drift — no longer applies. They are now just a bound on
case size.

**Pairing** is emission order, index by index. Phase 0 shows Blink paints blobs
in reading order, matching Parley. On failure *only*, both sides are re-sorted
by `(y, x, id)` and re-compared; if that passes, the report says "same glyphs,
different order" instead of a wall of position diffs. This is failure-path code
and hides nothing.

Under bidi, neither the pairing nor the emission order survives unexamined.
v1 has no bidi — Phase 1 established the bundled Roboto has zero `R`/`AL`
codepoints and asserts it.

**Styles** are compared as resolved `(postscript_name, font_size)` pairs per
glyph, never by index into the style table.

**`Mismatch`** is a typed value:

- `GlyphCount { parley, chrome }`, or
- `Glyphs { diffs: Vec<GlyphDiff>, total: usize, same_multiset: bool }`

where `GlyphDiff` carries the glyph index, both glyphs, both resolved styles,
`dx`/`dy` and the tolerance that was allowed. `Display` prints the totals and
the first 10 diffs; callers prefix the case identity, which `GlyphOutput` does
not carry.

## Bring-up gate

Run in order. Every step records its finding in this document. Nothing in
[regenerate](#srcbinregenerate_goldensrs) or [fuzz](#srcbinfuzz_looprs) runs
until all are green.

| # | question | probe | expected | if not |
| --- | --- | --- | --- | --- |
| B1 | Is chromedriver reachable and is `gpuBenchmarking` present? | The Phase 0 spike's probe script | `hasPrintToSkPicture: true` | `--allowed-ips` mismatch; set `CHROMEDRIVER_ALLOWED_IP` per the container README |
| B2 | Does the staged font load over `file://`? | `initHarness()` | `{ok: true, fonts: ["Roboto"]}` | confirm `--allow-file-access-from-files`; check the harness mount |
| B3 | Is `#content` at the document origin? | `getBoundingClientRect()` on `#content` | `(0, 0)` | amend `harness.js` to return the rect and subtract it in the driver — Parley reports layout-relative coordinates, Chrome document-absolute, and the comparison assumes they coincide |
| B4 | How many layers? | count `layer_*.skp` | exactly 1 | stop and investigate what promoted a second compositing layer |
| B5 | Is a capture ever stale? | render case A, capture, render case B, capture, assert the two JSON dumps differ | differ | if stale, a reused session lags by one case and looks like catastrophic parity failure; find a way to force a frame. If the two `rAF` waits prove unnecessary, they may be dropped |
| B6 | Which commands actually appear? | inventory command names across ~20 varied captures | `DrawPaint`, `DrawRect`, `DrawTextBlob` only | extend the ignored list for anything inert; a transform command means implementing the stack after all |
| B7 | What does `skp_parser FILE data/N` emit? | run it, `file(1)` the output | raw sfnt bytes on stdout | if it is text or hex, redirect into the bind mount via `docker exec sh -c` and/or decode; Phase 1 flagged this as unverified |
| B8 | Does `font-size: 0` neutralise Blink's strut? | single-run case; compare Chrome's glyph `y` against Parley's baseline | equal | **stop and report** — this needs a design decision (model the strut, or find another neutraliser). Do not work around it silently |
| B9 | Does Blink paint the hanging trailing space? | handwritten case that wraps at a space under `pre-wrap` | painted, matching Parley | drop the trailing-space glyph at the end of every non-final line on the Parley side, documented as a modelled Blink paint behaviour |
| B10 | Do combining marks produce `kFull`? | capture a case containing an `Mn` mark | either answer is fine | **check the JSON in as a fixture either way** — this replaces guessing at `kFull`'s shape |
| B11 | Is `textSize` omitted at exactly 12px? | capture a run at 12.0px | omitted | check the JSON in as a fixture regardless |
| B12 | Which 16.16 formula does Blink use? | see the [16.16 doc](./glyph-positioning-16-16-advances.md) | — | if inconclusive, restore the drift term and its line plumbing as the documented fallback |
| B13 | Does Blink round fragment origins to 1/64px? | after B12 lands: a case with several differently-sized runs, at sizes where one font unit is **not** a multiple of 1/64, and check whether residual error steps at `DrawTextBlob` boundaries | no residual | model it (`floor_to_layout_unit` already exists, unused, in the cases crate) — but note Parley's run boundaries need not coincide with Blink's blob boundaries, so this needs evidence before it is implemented |

B13 exists because removing the accumulation term exposes whatever the next
error source is, and `LayoutUnit`'s 1/64px grid is ~500× coarser than 16.16.
Phase 0's captures cannot separate the two hypotheses: at 24px with upem 2048 a
font unit is 3/256px, so exact accumulation lands on a 1/64 multiple anyway.
Choosing the probe's font sizes deliberately is the whole trick.

### Findings

Run 2026-08-13 against `parley-glyph-recorder:latest` (Chrome for Testing
151.0.7922.77) under Podman Desktop for macOS on arm64. B1–B7 and B9–B11 are
green; **B8 is red and is the one thing blocking the gate** — see
[the B8 finding](#b8-parleys-half-leading-floor) below. B12 and B13 are not yet
run: both are x-axis probes sequenced after the gate.

| # | finding |
| --- | --- |
| B1 | Green. `gpuBenchmarking` and `printToSkPicture` both present. `CHROMEDRIVER_ALLOWED_IP=192.168.127.1` was needed, exactly as `container/README.md` documents. |
| B2 | Green. `{ok: true, fonts: ["Roboto"]}` — the staged font loads over `file://`. |
| B3 | Green. `#content` is at `(0, 0)`, `scrollX`/`scrollY` are 0, and `body` is at `x: 0`. Layout-relative and document-absolute coordinates coincide, so no rect subtraction is needed. |
| B4 | Green. Exactly 1 layer, on every capture taken. |
| B5 | Green, but only after a **driver change** — see [the mount-coherency finding](#b5-never-delete-the-skp-from-the-host). Staleness itself is impossible: `PictureLayer::GetPicture` repaints from the live DOM on every call (`cc/layers/picture_layer.cc`), so a capture cannot lag a case. 8/8 captures were fresh. The two `rAF` waits are therefore unnecessary; they are kept because they cost nothing and the argument for dropping them rests on a Chromium implementation detail. |
| B6 | Green. Across 20 varied captures, only `DrawPaint`, `DrawRect` and `DrawTextBlob` appear. No transform or save/restore command, confirming Phase 0. |
| B7 | Green. `skp_parser FILE data/0` writes **raw bytes** to stdout — 171183 of them, starting `ff 00 05 90 01 01 06 "Roboto"`, i.e. a Skia wrapper, not the sfnt itself. `scan_for_valid_sfnts` finds **exactly one** valid sfnt, at offset 47, and `postscript_name_from_bytes` reads `Roboto-Regular`. The validating scan is doing real work here. |
| B8 | **Red.** The strut *is* neutralised, but Parley and Blink disagree on the baseline at 4 of 15 sampled sizes. See below. |
| B9 | Green. Blink **does** paint the hanging trailing space: a wrapped `"aaaa bbbb"` gives 9 glyphs on both sides, including the U+0020 glyph at the end of line 1, and every `x` agrees to 6 significant figures. No trailing-glyph dropping is needed. |
| B10 | Green, and the answer is **`kHorizontal`, not `kFull`**. `"ź q̃ ẉ k̏"` (a standalone `Mn` after each base, none of which Roboto precomposes) still serialises as a flat position array, so every glyph in the run shares one `y`. The mark carries a zero advance and sits at exactly its base's `x`, and Chrome and Parley agree on all six glyphs to 6 significant figures. `kFull`'s shape therefore remains **unobserved**, and its fixture stays hand-written. |
| B11 | Green. At exactly 12px the run's `font` object has no `textSize` key at all (its keys are `edging`, `embeddedBitmapText`, `hinting`, `subpixelText`, `typeface`), so the 12.0 default is load-bearing. Checked in as a fixture. |

#### B5: never delete the SKP from the host

> **The finding below remains true; the workaround it prescribes is superseded.**
> There is no host-side bind mount for `/skp` any more — the agent is the only thing
> that ever touches it, from inside the same container process space this finding
> already required. See "Why the agent directory's mount is exempt from B5's
> pathology" in
> [`glyph-positioning-recorder-agent.md`](./glyph-positioning-recorder-agent.md#why-the-agent-directorys-mount-is-exempt-from-b5s-pathology).
> The measurement itself — host-side `unlink` leaving the container's view of a bind
> mount stale — is unchanged history and is not being re-litigated.

The driver's per-case cleanup **must run inside the container**, not on the host
side of the bind mount, even though both see the same directory.

Measured, with the delete as the only variable and visibility polled host-side:
deleting from the host gave 2 of 5 captures visible and 3 of 5 never visible
within 5s; deleting via `docker exec` gave 5 of 5, every one visible on the
first poll (~0.3ms). Writing to a container-local path instead of the mount made
no difference, which rules the bind mount itself out as the cost — a host-side
`unlink` leaves the container's view of the directory entry stale, and the SKP
Chrome writes next is then frequently never visible to the host at all.

This supersedes step 1 of [`driver.rs`](#srcdriverrs)'s per-case list. It also
justifies that section's "no polling": once the delete is container-side, the
file is there by the time the driver hears back, as claimed.

This is a Podman-on-macOS finding. It is very likely benign on real Docker, but
the container-side delete is correct everywhere and costs one `docker exec`, so
it is not worth branching on.

#### B8: Parley's half-leading floor

`font-size: 0` **does** neutralise Blink's strut. Chrome's baseline is exactly
`round(hhea_ascent × size)` at all 15 sampled sizes, with no contribution from a
container strut and — a **correction to the Phase 1 correction** — with the
FreeType ascent/descent hack **not firing at all**. Phase 1 concluded the hack
was live because `subpixelText: true` appears in the captures; that is `SkFont`'s
flag, not `FontRenderStyle::use_subpixel_positioning`, which is what
`font_metrics.cc` actually gates on. Had it fired, the baseline would be one
less at 10/13/14/18/21/22/26/30px; it is not, at any of them.

The divergence is on the Parley side. `LineBoxExtents::add_text`
(`parley/src/layout/line_break.rs:123-145`) computes:

```rust
let (ascent, descent) = (metrics.ascent.round(), metrics.descent.round());
let half_leading = (line_height - (ascent + descent)) / 2.;
let over = ascent + half_leading.floor();
```

For CSS `normal`, `line_height` is the **unrounded** `ascent + descent + gap`,
but it is then reduced by the **rounded** ascent and descent. Whenever rounding
both up overshoots the unrounded total, `half_leading` is a small negative
number, `floor` takes it to `-1`, and the baseline lands a whole pixel above
Blink's. Blink has no such term: it derives `normal` line height from the
already-rounded metrics, so its half-leading is exactly zero.

Measured for Roboto, `chrome_y − parley_y`:

| size | 10 | 11 | 12 | 13 | 14 | 16 | 18 | 20 | 21 | 22 | 24 | 26 | 28 | 30 | 17.23 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Δ | 0 | **1** | 0 | 0 | 0 | **1** | 0 | **1** | 0 | 0 | 0 | 0 | **1** | 0 | 0 |

The formula reproduces every value exactly, including the size-11 case where
Parley's baseline (9) is a full pixel below even `floor(ascent)` (10).

Roughly a quarter of the 10–30px sampling range is affected, so a recorded
corpus would fail on `y` for a large fraction of its cases. Per this step's
stated contingency this was escalated rather than worked around; the resolution
is to fix it in Parley — `LineHeight::MetricsRelative` is simply not quantized
alongside the metrics it is measured against.

#### B12/B13 preview: the x error is a *scale*, not an accumulation

A 20-case `fuzz_loop` run (seeds 2000000+, before the sampling change below)
found `x` diverging far more than the plan budgeted for, and in the wrong shape.

The plan's model is 16.16 accumulation: error bounded by `index × 2⁻¹⁶`, i.e.
≤1.5e-5 px per glyph, growing with a glyph's *index within its line*. What the
captures show instead is a constant **relative** error — the residual is
proportional to `x`, not to the index:

| size | measured `(parley_x − chrome_x) / x` |
| --- | --- |
| 24.08px | 6.8e-5, constant across the line |
| 10.42px | 1.33e-3, constant across the line |

At 10.42px that reaches 0.06px by the end of a line, ~100× the accumulation
term. A relative error is a *scale* difference, and both figures are close to
what `floor(size × 64) / 64` predicts (7.8e-5 and 1.32e-3): Blink is shaping at
a size quantized to `LayoutUnit`'s 1/64 px grid, on top of the 1/100 px
truncation the extract crate already models.

The response is to **remove the confound rather than model it**: font sizes are
now sampled on the **1/4 px grid**, where the 1/64 and 1/100 grids intersect, so
neither quantization moves a generated size. See `FONT_SIZE_STEP` in
`glyph_positioning_cases/src/generate.rs`. This supersedes Phase 1's
1/100-grid-plus-offset sampling, whose purpose was to exercise the 1/100
truncation — a smaller effect than the one it was hiding.

A second 30-case run on the new grid (seeds 3000000+) confirms it: **18 of 30
cases now have no `x` failure at all**, with per-glyph residuals at ±5e-5, i.e.
exactly the 6-significant-figure serialisation floor and nothing more. The
16.16 accumulation term is not visible above that floor. Every case still fails
on `y`, from B8.

What remains for B12 is therefore the residual *after* this change, and on this
evidence there may be nothing left for it to model. The hypothesis set in
[the 16.16 doc](./glyph-positioning-16-16-advances.md) is untested, and its
framing — that 16.16 accumulation is the dominant term — is now known to be
wrong at the sizes Phase 1 sampled: it was masked by a size-quantization error
two orders of magnitude larger. **Re-derive that doc's premise before acting on
it**, and do not change `parley_engine` on the strength of it alone.

The remaining `x` failures fell into two shapes:

- **Run-boundary steps of ~1/64 px.** A glyph at a font-size change came out at
  Chrome's `78.609375` (exactly `5031/64`) against Parley's `78.59448`, and a
  line-final space at Chrome's `33.0` (exactly `2112/64`) against Parley's
  `32.992676`. This is B13's hypothesis, and it is now **resolved from source**
  rather than inferred from captures: `ShapeResult::SnappedWidth()` and
  `ShapeResultView::SnappedWidth()`
  (`third_party/blink/renderer/platform/fonts/shaping/{shape_result.h:170,
  shape_result_view.h:124}`) are both `LayoutUnit::FromFloatCeil(width_)` — an
  explicit **ceiling**, distinct from the plain `LayoutUnit(float)` constructor
  used elsewhere (which truncates toward zero) and from `FromFloatRound`. This
  is what places every fragment after the first on a line, so Chrome's position
  after a run boundary is always ≥ the unrounded value. Not yet modelled on the
  Parley side; the residual after the fix below tops out at 0.023px, consistent
  with this and nothing larger.
- **Whole-line uniform offsets** of 0.35–4.3 px, constant across every glyph
  from some point in the line onward. Diagnosed, not a line-breaking issue: see
  below.

#### A real Parley bug, found via the "uniform offset" cases

The offset in every such case is **numerically exactly the run's
`letter_spacing` or `word_spacing`** (e.g. `dx = +2.468735` against
`letter_spacing = 2.46875`; `dx = +1.796844` against `word_spacing =
1.796875`) — not a coincidence, confirmed across half a dozen cases for both
properties. Controlled captures isolate the trigger: plain ASCII with
letter-spacing, ASCII with a space, word-spacing alone, and a simple
base+single-combining-mark pair are all clean. It reproduces only for a
character that itself requires font-level decomposition (no precomposed glyph
in Roboto, so its own accent is synthesized under the base's `HarfBuzz` cluster
id) followed by a *separate* explicit combining-mark codepoint (its own,
different cluster id) — e.g. `Ŝ` (no accent-free base glyph) followed by
U+0301, or `ΰ` (already accented) followed by U+0303.

Root cause: `parley_engine`'s shaped-cluster boundary detection
(`parley_engine/src/shape/shaped_text.rs:435-525`) groups HarfBuzz output
glyphs into "shaped clusters" purely by matching `glyph_info.cluster` (a byte
offset). In the trigger shape above, the base's synthesized accent glyph shares
the base's cluster id, but the following explicit mark has its own — so Parley
splits one grapheme into two shaped clusters. `LayoutData::finish`
(`parley/src/layout/data.rs:341-368`) then adds letter/word spacing once per
shaped cluster, so that grapheme gets it twice. Word spacing is exposed too
whenever this split happens to land next to a whitespace cluster.

This is a genuine `parley_engine` bug, unrelated to the harness, and out of
scope to fix here. **Both `LETTER_SPACING_RANGE_PX` and
`WORD_SPACING_RANGE_PX` are temporarily pinned to `(0.0, 0.0)`** in
`glyph_positioning_cases/src/generate.rs` to unblock corpus recording; their
doc comments record the recommended ranges to restore (`(-0.5, 2.5)` and
`(-1.0, 4.0)`, Phase 1's originals) once the cluster-merging bug is fixed. Do
not restore them before that fix lands — it just reintroduces cases the corpus
can't pass.

With that change, a fresh 50-case fuzz run's worst-case residual dropped from
several pixels to 0.023px — consistent with only the ceil-rounding item above
remaining.

## Golden file format amendment

`Golden` gains `note: Option<String>`, written as a `note <text>` line
immediately after `seed` and omitted when absent. It exists so a handwritten
case can say why it exists; the format has no comments.

## `src/bin/regenerate_goldens.rs`

**Recording only.** It never compares against Parley — Chrome is the oracle and
`cargo test` is the place a verdict is rendered.

One uniform rule: **every existing file under
`parley_tests/tests/glyph_positioning/**/*.txt` has its `Case` re-recorded in
place**, preserving `seed` and `note`. Handwritten cases are therefore authored
by hand with `styles 0` / `glyphs 0` and filled in by the first run; promoted
fuzz regressions already arrive in exactly that shape.

The one addition on top of that rule: `generated/` is also populated from
`Case::from_seed(0..GENERATED_SEED_COUNT)` (50), written as
`generated/seed_0000.txt`. Files in `generated/` outside that range are
reported as stale, not deleted.

Takes an optional path filter, so iterating on one case does not re-record 50.

Re-running must produce byte-identical files. That idempotency check is Phase
5's verification step and is the real test of Chrome's determinism.

## `src/bin/fuzz_loop.rs`

Keeps one session alive across many cases.

- **Seeds** come from a range disjoint from the golden set: `>= 1_000_000`. The
  base is derived from the wall clock (no `rand` dependency) and **logged**, and
  can be pinned with `--start-seed`, so any hit is reproducible.
- **On mismatch**, write a directory under `--out`
  (default `target/glyph_positioning_fuzz/`) containing: the golden-format file
  (Case + Chrome output — already promotable by copying it into
  `regressions/`), the `Mismatch` `Display` output, the raw `skp_parser` JSON,
  and the `.skp` itself. Keeping the last two matters because a deserializer bug
  and a genuine parity bug look identical in the diff alone. Then **continue**.
- **On a WebDriver error or script timeout**: close the session, open a fresh
  one, re-navigate, re-run `initHarness`, retry the case once. Two consecutive
  failures on the same case records a harness-failure artifact and moves on;
  five consecutive failures overall aborts with a clear message. There is no
  proactive periodic recycling — add it if a run shows drift or leaks, not
  before.
- Stops on `--max-cases` or Ctrl-C, printing counts and a rate.

Promotion is manual, mirroring `PARLEY_TEST=accept`: copy an artifact's case
file into `tests/glyph_positioning/regressions/<name>.txt`.

## Dependencies and errors

New dependencies: `fantoccini` and `tokio` (`rt`, `macros`, `time` — one
sequential session needs no multi-thread runtime). `serde_json` is already
present.

**No `anyhow`, no `thiserror`.** Errors are
`Box<dyn std::error::Error + Send + Sync>` behind a crate-local `Result<T>`
alias, with context added by hand where it helps (`case 37: capture timed out`).
The workspace has no error crate today and a test harness is not where the first
one should arrive. This is what the Phase 0 spike used.

## Tests

Everything in `skp_json.rs` is testable without Docker, and it is the only part
of Phase 4 that is:

- **Real captures**, checked in under `tests/fixtures/` during bring-up: the
  Phase 0 `layer_0` dump (asserting its exact 4 blobs and their glyph
  positions), the B10 combining-mark capture, and the B11 12px capture. Real
  data rather than hand-written JSON is what stops `kFull`'s shape from being a
  guess.
- **Synthetic fixtures only for rejection paths that cannot be captured** with
  one font and no transforms: two distinct `typeface` keys, a `Concat44` in the
  stream, a `kDefault` run, `visible: false`. Hand-written input is fine there
  because the assertion is that we refuse it, not that we interpret it.

## Verification

- `cargo nextest run --workspace` passes with no Docker and no network.
- `cargo clippy --workspace --all-targets --all-features` is clean; the
  wasm/android matrix jobs still succeed with the recorder excluded (already
  wired in Phase 2).
- The bring-up checklist is green and its findings are written into this file.
- A short `fuzz_loop --max-cases 100` run completes, and any mismatches it
  reports are real rather than harness artifacts.
- The 16.16 doc's blast-radius measurement is recorded.

## Delegation

Per the parent plan, implement via Sonnet subagents where the work is
separable — `skp_json.rs` plus its fixtures is the cleanest independent chunk,
and `compare` is another. The bring-up checklist is **not** delegable: it is
where guesses get made. If a Docker or permission wall is hit, stop and wait for
user input; do not bypass it.
