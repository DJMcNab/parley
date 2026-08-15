# Modelling Blink's 16.16 advance quantization in Parley

A spike carried by Phase 4 of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md),
but aimed past it: the divergence it addresses is a standing source of
Parley-vs-Chrome width differences in real products, not just a nuisance for
this harness.

Sequenced **after** the Phase 4 recorder can capture, since the whole point is
to pick the model from ground truth rather than from reading Chromium.

## The divergence

Parley shapes in font units and scales once, in f32
(`parley_engine/src/shape/shaped_text.rs:513-519`):

```rust
let glyph = Glyph {
    id: glyph_info.glyph_id,
    x: (glyph_pos.x_offset as f32) * scale_factor,
    y: -(glyph_pos.y_offset as f32) * scale_factor,
    advance: (glyph_pos.x_advance as f32) * scale_factor,
};
```

with `scale_factor = font_size / units_per_em`. Advances are therefore
arbitrary f32 values, and a line's width is their f32 sum.

Blink never sees such a number. It runs HarfBuzz at a 16.16 fixed-point scale,
so **every advance is already a multiple of 1/65536 px** before Blink
accumulates anything, and Blink then accumulates in `TextRunLayoutUnit`
(`FixedPoint<16, int32_t>`), the same grid. The quantization is systemic, not
incidental.

Phase 0's spot-check missed this entirely, by coincidence: at 24px with upem
2048 every advance is `units × 3/256`, which is exactly representable in 16.16.
At a size like 17.23px it is not.

## Why the harness is the right place to settle it

The recorder is the only thing that can *validate* a model against ground
truth. Reading Chromium yields a plausible formula; a corpus of captures yields
a formula that demonstrably reproduces Chrome to the serialisation floor. Once
the model is in place, **every golden becomes a live proof it is exactly
right** — a wrong model fails loudly, at scale, on the next regeneration.

## Hypothesis set

`u` = advance in font units, `s` = CSS font size already truncated to 1/100px
(which the extract crate applies today), `e` = units per em.

| # | formula |
| --- | --- |
| H1 | `round(u × s / e × 65536) / 65536`, product in f32 |
| H2 | as H1, product in f64 |
| H3 | `floor(...)` of H1 |
| H4 | `floor(...)` of H2 |
| H5 | integer: `scale16 = round(s × 65536)`; `(u × scale16 + e/2) / e` then `/65536` |
| H6 | HarfBuzz `em_mult`: `mult = (scale16 << 16) / e` (truncating); `(u × mult + 32768) >> 16` |

H1/H2 are the likeliest: Blink installs its own HarfBuzz font funcs backed by
`SkFont`, so the advance arrives as a Skia float and is converted to a
`hb_position_t` by a single scalar→16.16 rounding. H6 is the shape HarfBuzz
uses when reading `hmtx` itself, and is listed because it is what applies if
Blink's font funcs are not in play on this path. **This list is a starting
point for the probe, not a claim about Chromium's source.**

## The probe

Two independent discriminators, both run against Phase 4 captures.

**Direct, at small x.** `skp_parser` emits 6 significant figures, so its
half-ulp is ~5e-4 px at x≈200 — far larger than the 1.5e-5 px quantum. The
hypotheses only separate where the ulp is smaller than the quantum, i.e. the
first few glyphs of a run: at x<10 the ulp is ~5e-6, at x<1 it is ~5e-7.
Generate cases whose runs are short, or read only the leading glyphs of longer
ones.

**Aggregate bias.** Rounding is unbiased; flooring is not. Over a few thousand
glyphs, the mean residual `parley − chrome` separates the round and floor
families decisively even where individual glyphs are ambiguous. This is the
stronger test and does not depend on small x.

Choose font sizes for which `u × s / e` is *not* exactly representable in
16.16 — that is, avoid sizes where `s/e` is a dyadic rational with a small
denominator. At upem 2048, sizes that are multiples of 1/8 are exactly the
degenerate case (24px is one, which is why Phase 0 saw nothing).

If the probe cannot separate the hypotheses, the documented fallback is to
restore Phase 1's `i × 2⁻¹⁶` drift tolerance and the line plumbing it needs
(see Phase 4's B12).

## The change

Apply the winning formula **unconditionally** at
`parley_engine/src/shape/shaped_text.rs:513-519`, to `advance` **and** to the
GPOS `x`/`y` offsets. Offsets are in scope because Phase 1 deliberately keeps
combining marks in the sampling alphabet, and a mark's entire position is a
GPOS offset.

Watch the sign: `y` is negated on the way from font space to layout space, so a
round-half-up rule is not symmetric under that negation. Quantize before
negating, or use a symmetric rule — whichever matches the captures.

No feature flag and no runtime option, by decision: the goal here is to **see
the blast radius**, and gating it hides exactly that. Splitting it out — as an
option, a feature, or an upstream RFC — is a later decision informed by what
the measurement shows.

## Measuring the blast radius

1. `cargo nextest run --workspace` before and after. Record which tests move.
2. The PNG snapshot tests in `parley_tests` are the expected casualties. Run
   them with `PARLEY_TEST=accept` on a scratch branch to see the *magnitude* of
   the change — do **not** commit accepted snapshots as part of the
   measurement.
3. Record, in this file: the list of affected tests, the largest positional
   delta observed, and whether any change is visible rather than sub-pixel.

That record is the input to the gating decision, and to any conversation about
upstreaming this to Parley proper.

## Follow-ups this spike does not do

- Gating, or an upstream API for choosing the quantization model.
- Blink's own CSS-size → `SkScalar` → `hb_font_set_scale` chain (a second
  quantization of the *size*, on top of the 1/100px truncation the extract
  crate already models). Likely below the serialisation floor; revisit if
  residuals demand it.
- Fragment-origin rounding to `LayoutUnit`'s 1/64px grid, which is the next
  suspect once this term is gone. Tracked as Phase 4's B13, not here.
