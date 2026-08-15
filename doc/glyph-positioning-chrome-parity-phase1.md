# Phase 1 — case generation and Parley-side extraction

Implementation spec for Phase 1 of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md).
Read the parent plan first for the overall architecture; this file is the
authority on what Phase 1 builds and why.

Phase 1 delivers **two crates and no browser interaction at all**. Everything
here is offline, deterministic and testable without Docker.

Several decisions taken while writing this spec bind *other* phases (harness
CSS in Phase 2, `skp_parser` parsing in Phase 4). Those have been folded back
into the parent plan's decision list rather than duplicated here; this file
only restates them where Phase 1 code depends on them.

## Deliverables

| crate | directory | dependencies |
| --- | --- | --- |
| `parley_glyph_positioning_cases` | `parley_tests/glyph_positioning_cases` | `rand`, `rand_chacha`, `read-fonts` |
| `parley_glyph_positioning_extract` | `parley_tests/glyph_positioning_extract` | `parley`, `fontique`, the cases crate |

The name `parley_glyph_positioning_extract` is provisional; rename freely at
review time.

The split exists so the data crate never depends on Parley. The extract crate
is the *only* place the Parley side of the comparison is constructed, and both
the Phase 5 CI test and the Phase 4 fuzz loop call into it — if the style
setup were duplicated in those two places, a drift between them would silently
invalidate every golden.

The parent plan placed `parley_extract.rs` inside the cases crate and named
only two crates. That is superseded.

### `parley_glyph_positioning_cases`

- `FONTS: &[SupportedFont]` registry with `include_bytes!`, mirroring
  `parley_linebreaking_cases::FONTS`. v1 has one entry: the bundled
  `Roboto-Regular.ttf`.
- `Case` / `Run` and `Case::from_seed`.
- The per-font sampling alphabet, computed at runtime from the font's `cmap`
  and cached behind a `OnceLock`.
- Chromium quantization helpers (font-size truncation; the 1/64px grids).
- `hack_would_fire`.
- The golden output schema plus its hand-rolled text reader/writer.
- A validating sfnt scan + PostScript-name lookup, used by Phase 4 against
  Chrome's serialized typeface blob and by the extract crate against the
  Parley-selected font.

### `parley_glyph_positioning_extract`

- `fn layout(case: &Case, font_cx: &mut FontContext, layout_cx: &mut LayoutContext<()>) -> Layout<()>`
- `fn parley_output(layout: &Layout<()>) -> GlyphOutput` — accumulating glyph x
  in **f64**, not via `positioned_glyphs()`. See "Comparison predicate".

## The `Case` grammar

```rust
pub struct Case {
    /// Provenance only. Never used to regenerate the case at test time.
    pub seed: u64,
    pub runs: Vec<Run>,
    /// CSS px. Always an exact multiple of 1/64.
    pub width: f32,
}

pub struct Run {
    pub text: String,
    /// CSS px, pre-truncation. See "font size" below.
    pub font_size: f32,
    /// CSS px. Always an exact multiple of 1/256.
    pub letter_spacing: f32,
    /// CSS px. Always an exact multiple of 1/256.
    pub word_spacing: f32,
}
```

There is no `line_height` field. v1 fixes line height to CSS `normal`, which
is Parley's default `LineHeight::MetricsRelative(1.0)`. The parent plan's
`LineHeightValue` is dropped.

`font_size` is **per-run**, not per-case. This means the line box is a union of
differing rounded ascents/descents across runs, which is deliberately in scope.

## Generation

`Case::from_seed(seed: u64)` uses `ChaCha8Rng::seed_from_u64`, exactly as
`parley_linebreaking_cases::Case::from_seed` does.

### Alphabet

Per font, the sampling alphabet is `cmap ∩ BMP` minus four hazard classes,
expressed as a hand-written const range list. **No Unicode character database
dependency is needed** — these ranges are stable and font-independent, so the
categories are resolved once at authoring time:

| class | ranges |
| --- | --- |
| `Cc` | `0000–001F`, `007F–009F` |
| `Zs` | `0020`, `00A0`, `1680`, `2000–200A`, `202F`, `205F`, `3000` |
| `Cf` | `00AD`, `0600–0605`, `061C`, `06DD`, `070F`, `0890–0891`, `08E2`, `180E`, `200B–200F`, `202A–202E`, `2060–2064`, `2066–206F`, `FEFF`, `FFF9–FFFB` |
| `Co` | `E000–F8FF` |

A conservative superset is fine — excluding a handful of extra codepoints
costs nothing. Sampling is **uniform** over the result.

For the bundled Roboto this yields **874 codepoints** from 896 cmapped
(22 excluded), compressing to 72 contiguous ranges. Coverage is Cyrillic 255,
Latin-Ext 146, LatinExtAdd 100, ASCII 98, Latin-1 96, Greek 75. Two properties
fall out for free and should be relied on but also asserted:

- **No RTL** — the font has zero bidi `R`/`AL` codepoints, so v1 has no bidi.
- **No complex scripts** — no Devanagari/Thai/Arabic, so no complex shaping.

The 10 `Mn` and 2 `Me` combining marks are **kept**. They exercise GPOS mark
attachment and zero-advance glyphs, and are safe because neither Blink nor
Parley normalizes (Parley's only `compose` code is IME preedit; there is no
`icu_normalizer` use in layout).

### Spaces

U+0020 is excluded from the alphabet and inserted by a separate rule, subject
to two constraints:

- **No two adjacent U+0020**, including across run boundaries. Under
  `pre-wrap` CSS hangs an entire trailing space *sequence* at a break, whereas
  Parley hangs exactly one space and pushes the rest to the next line.
- **No run begins with a space.** `ShapeResultSpacing::ComputeSpacing` skips
  word spacing at `index == 0` unless the character is NBSP.

Leading and trailing spaces on the whole `Case` *are* legal, unlike the
line-breaking corpus, because `pre-wrap` preserves them.

### Font size

Per-run, using `parley_linebreaking_cases`' scheme unchanged:
`10.0 + k × 0.02 + offset` where `offset ∈ [0, 0.0095)`. Truncating to 1/100px
therefore always lands on an *even* hundredth, so no two sizes are ever
adjacent hundredths. This dodges Chrome's order-dependent font-cache collision
(where e.g. 16.79 and 16.80 share a key and the first used defines the other)
— which now matters both *within* a case, since runs carry different sizes,
and *across* cases, since the fuzz loop reuses one browser session.

The stored `font_size` is the untruncated value; the harness sets it verbatim
in CSS and the extract crate applies the truncation. That way the truncation is
exercised rather than assumed.

### FreeType hack pre-filter

Any run size for which `hack_would_fire` returns true is **rerolled from the
same RNG stream**, preserving a gapless seed→case mapping. The condition,
confirmed verbatim from `font_metrics.cc` at tag `151.0.7922.77`:

```cpp
// Linux / ChromeOS / Android / Fuchsia only
if (use_subpixel_positioning && descent < SkScalarToFloat(metrics.fDescent) && ascent >= 1) {
  ++descent;  --ascent;
}
```

`descent < fDescent` after `SkScalarRoundToScalar` means the fractional part of
the descent in physical px lies in `(0, 0.5)`. So:

```rust
fn hack_would_fire(hhea_descender: i16, units_per_em: u16, css_font_size: f32) -> bool
```

is `frac(|hhea_descender| × size / upem) ∈ (0, 0.5)`, plus `round(ascent) >= 1`.

We filter rather than model because the hack is **Linux-only** and the real
target is Windows/macOS Chrome — a firing case would only ever validate an
artefact of the stand-in oracle. The cost is that roughly **half** the 10–30px
range is unreachable for Roboto (descent = `size × 0.244140625`; 10px and 30px
fire, 12/16/20/24px do not), so the golden size distribution is visibly
non-uniform. That is accepted.

### Spacing

`letter_spacing` and `word_spacing` are sampled as exact multiples of **1/256**.

Blink stores both as `TextRunLayoutUnit`, which `layout_unit.h` defines as
`FixedPoint<16, int32_t>` — i.e. **1/65536 px** — and `ComputeSpacing` adds
them per glyph with no further rounding. The per-glyph error would accumulate
to ~1.5e-3 px over a 100-glyph line, roughly 30× the comparison tolerance at
x≈200px, so it cannot be absorbed. Sampling on a coarser binary grid makes the
float→`TextRunLayoutUnit` conversion lossless, which sidesteps the question of
whether that conversion rounds, floors or truncates. That question is
**deliberately left unresolved**; lifting the grid restriction later requires
answering it.

### Shape

- 1–4 runs per case.
- 30–120 characters of text in total.
- `width` = (largest run `font_size`) × a factor in `[7, 32]`, matching
  `linebreaking_cases`, then **capped so no glyph x exceeds ~1000px** and
  snapped down to a multiple of 1/64.

The 1000px cap keeps the comparison tolerance (below) around 30× tighter than
one Blink 1/64px subpixel. The 1/64 grid means the CSS px width survives
Blink's `LayoutUnit` (`FixedPoint<6, int32_t>`) conversion losslessly.

## Chromium behaviour modelled

| behaviour | model | source |
| --- | --- | --- |
| `font-size` truncated to 1/100px | `(size × 100.0).floor() / 100.0` | already in `linebreaking_matches_chrome.rs` |
| line fits against available width + 1/64px | pass `width + 1/64` | `AvailableWidthToFit`, `line_breaker.h` |
| Chrome accumulates advances in 16.16, Parley in f32 | *breaking:* a further `+ 1/64`; *positions:* the `i × 2⁻¹⁶` tolerance term, and f64 accumulation on the Parley side | `RESIDUAL_SLACK_SUBPIXELS` |
| ascent/descent rounded to whole px | `quantize: true` on the builder | `line_break.rs:123–144` already does this |
| spacing quantized to 1/65536px | avoided by grid sampling | `TextRunLayoutUnit`, `layout_unit.h` |
| FreeType ascent/descent ±1 | avoided by pre-filter | `font_metrics.cc` |

`layout()` therefore passes `max_advance = case.width + 2.0/64.0`
**unconditionally**. `linebreaking_matches_chrome.rs` measured that ~1.5% of
cases need the second epsilon, and its own doc comment endorses applying the
margin unconditionally in code that must match Chrome. Here the stakes are
higher than for line-break parity: a break in the wrong place makes every
glyph on every subsequent line mismatch. The tradeoff is that a genuine
sub-1/64px break divergence would be masked.

Blink reads **`hhea`** metrics, confirmed empirically: the captured baseline is
`y = 22` at 24px Roboto, and `round(1900 × 24 / 2048) = 22`, whereas `OS/2`
`win` gives 23 and `typo` gives 18. `skrifa` also picks `hhea` here because the
font has `USE_TYPO_METRICS` off. **A font with `USE_TYPO_METRICS` set would
invalidate this assumption** and must be re-checked before being added.

## Golden output schema

```rust
pub struct GlyphOutput {
    /// Deduplicated in first-appearance order.
    pub styles: Vec<Style>,
    pub glyphs: Vec<PositionedGlyph>,
}

pub struct Style {
    pub postscript_name: String,
    pub font_size: f32,
}

pub struct PositionedGlyph {
    pub id: u32,
    /// Absolute, in CSS px.
    pub x: f32,
    pub y: f32,
    pub style: u16,
}
```

**There is no line concept.** Chrome exposes a true per-glyph `y` (see the
parent plan's Phase 4 notes on `positions`/`coords`), so no line grouping needs
inferring — and the schema then survives future vertical-align work, where
glyphs on one line legitimately sit on different baselines. Line grouping may
be re-derived best-effort in failure *reporting*, never in the schema.

`postscript_name` is `name` ID 6. On the Parley side it is read from the
selected font with `read-fonts`; on the Chrome side Phase 4 extracts the
serialized typeface and runs the same lookup.

**Precondition:** every font offered to a `Case` must have a distinct
PostScript name. v1 additionally asserts that a capture contains **exactly one**
distinct name, so a silent fallback to a system font fails loudly with a clear
message instead of surfacing as garbled glyph IDs.

### Trailing whitespace

Nothing is stripped on either side. `white-space: pre-wrap` makes Blink hang
the overflowing space exactly as Parley does — `line_break.rs:834–852` appends
the space atom to the current line past `max_advance`, then breaks. Whether
Blink actually *paints* that hanging glyph is unverified; see below.

## File format

Hand-rolled compact text, following the precedent of
`linebreaking_matches_chrome.rs`, which hand-parses CSV specifically to avoid
pulling in a dependency. **No serde in either Phase 1 crate** — `serde_json` is
confined to the Phase 4 recorder, which is native-only and excluded from the
wasm/android CI matrix, and which needs it anyway for `skp_parser` output.

Every golden stores the **full `Case`** alongside the Chrome output, in one
uniform format shared by handwritten, regression and generated cases. The seed
is recorded as provenance only and is **never** used to regenerate a case at
test time.

This supersedes the parent plan's seed-only storage for generated cases. The
size argument for seed-only does not hold: the output is ~200 glyphs × ~20
bytes ≈ 4 KB per case, against a `Case` of ~550 bytes — about 13% on top of
data that must be committed regardless. In exchange, seed-only carries a sharp
failure mode, since any change to `from_seed` silently repoints every golden at
a different case while the suite keeps passing.

## Comparison predicate

> **Superseded in part by Phase 4.** Term 2 below (the `i × 2⁻¹⁶` accumulation
> tolerance) and its `index_in_line` argument are **removed**: Phase 4 models
> Blink's 16.16 quantization on the Parley side instead of tolerating it, so
> the predicate collapses to Term 1 alone on both axes. The reasoning in Term 2
> is why that spike exists and is kept for that reason. See
> [`…-phase4.md`](./glyph-positioning-chrome-parity-phase4.md) and
> [`glyph-positioning-16-16-advances.md`](./glyph-positioning-16-16-advances.md).
> `compare` also moved to the cases crate rather than the recorder.

Two independent error sources, both modelled rather than banded away. The
predicate lives in Phase 1 (it is part of the schema contract) even though the
diffing code that uses it is Phase 4's `compare.rs`.

```
|parley_x − chrome_x| ≤ half_ulp_6sig(chrome_x) + i × 2⁻¹⁶
```

where `i` is the number of glyphs preceding this one **within its line**.

### Term 1 — `skp_parser` serialisation

`skp_parser` emits at most **6 significant figures** (verified across all six
Phase 0 captures; the widest value seen is `-17.6836`), so exact match as the
parent plan words it is unachievable. This term models a known lossy
serialisation step rather than being fuzzy banding, and it shrinks
automatically as positions shrink.

### Term 2 — 16.16 advance accumulation

Blink runs HarfBuzz at 16.16 scale, so **every glyph advance is quantized to
1/65536 px before accumulation**, systematically narrower than Parley's f32.
Phase 0's spot-check missed this only by coincidence: at 24px with upem 2048
every advance is `units × 3/256`, which *is* exactly representable in 16.16. At
a sampled size like 17.23px it is not.

The error is biased (truncation, not rounding), so the bound grows linearly:

| glyphs into the line | worst-case drift |
| --- | --- |
| 30 | 4.6e-4 px |
| 60 | 9.2e-4 px |
| 120 (our cap) | 1.8e-3 px |
| ~1000 | 1.5e-2 px ≈ 1/64 |

**A flat 1/64px tolerance was considered and rejected.** 1/64 is roughly the
drift bound for a *1000-glyph* line, ~9× looser than our 120-glyph cap needs —
and it is larger than one font unit at every size in the sampled range
(4.9e-3 px at 10px, 9.8e-3 at 20px, 1.47e-2 at 30px, and exactly 1/64 at 32px).
It would therefore silently swallow a wrong kern, a wrong advance, or a mark
placed one unit off, which is a large fraction of what this harness exists to
find. The drift-proportional form keeps a one-unit error detectable at every
size while holding early glyphs to a near-exact standard.

**Note:** the parent plan says to *"reuse/port the logic already proven in
`linebreaking_matches_chrome.rs`"* for 16.16 accumulation. **No such logic
exists** — that test does not model 16.16 at all, it tolerates it via
`RESIDUAL_SLACK_SUBPIXELS`. There is nothing to port.

### Parley must accumulate in f64

`parley_output` must **not** reuse `GlyphRun::positioned_glyphs()`, which
accumulates `offset += g.advance` in f32. At 120 glyphs near x≈1000px that
carries a worst-case error of ~7e-3 px — larger than the 16.16 drift and close
enough to 1/64 to defeat the point of a tight predicate. Accumulating the same
per-glyph advances in **f64** reduces that term to negligible, leaving Chrome's
16.16 drift as the only one that needs a tolerance.

The per-glyph advances themselves remain f32; that is inherent to Parley and is
exactly what we want to measure. Only the accumulator changes.

### Vertical

`y` has no accumulation term — baselines derive from rounded ascents and are
whole pixels — so y is compared with the `half_ulp_6sig` term alone, i.e. very
nearly exactly.

### Eventual target

Modelling 16.16 on the Parley side (re-accumulating in `i64` 16.16 units so the
drift disappears entirely, rather than being tolerated) is the stronger
end state and would drop the predicate back to the serialisation floor. It
needs Blink's exact per-advance quantization mode — round vs floor vs truncate
— which we do not know and can only determine from Phase 4 captures. Revisit
once real data exists; note that tightening the predicate later may require
regenerating goldens.

## Tests

Phase 1 ships a **determinism test only**: `Case::from_seed(s) == Case::from_seed(s)`
over a range of seeds, in the style of `parley_linebreaking_cases::well_formed`.

Broader property tests (alphabet membership, the space constraints, spacing
grid, `hack_would_fire` never true for a generated size, sizes on the
even-hundredth grid) and a golden-format round-trip test are deliberately
deferred rather than written speculatively — they are cheap to add once real
data exists and shows which invariants actually matter.

## Unverified, to resolve in later phases

- Does Blink paint the hanging trailing space under `pre-wrap`? Phase 4.
- Does container `font-size: 0` neutralise the strut without tripping a
  different Blink code path? Phase 4.
- Does `skp_parser FILE.skp data/N` write raw bytes to stdout, or text?
- The float → `TextRunLayoutUnit` rounding mode, sidestepped by grid sampling.
- Whether `USE_TYPO_METRICS` fonts break the `hhea` assumption. They are not
  used in v1.

## Out of scope for v1

Multiple fonts, weight, italic (synthetic oblique would also hit an upstream
`skp_parser` bug — `scaleX` and `skewX` are written under the same JSON key),
non-`normal` line heights, bidi, complex scripts, vertical-align, inline boxes,
and the strut. The schema and the `FONTS` registry are shaped so multi-font is
an extension rather than a rewrite.
