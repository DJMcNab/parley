# Phase Four — Parley API-change Alternative #1: pair positioned glyphs with their source char

> Required by PLAN.md ("think through at least two possible API changes, make plans for
> both"). **This is an alternative to the recommended zero-change Plan A** (see
> `phase_four_plan.md` §5). Choose Plan A unless the team decides the ergonomic/robustness
> gains here are worth a (small, additive) Parley change.

## Motivation

Under Plan A the renderer recovers each emoji's codepoint by walking `Run`/`GlyphRun`
clusters (`Cluster::source_char()`) and re-deriving each cluster's `x` by accumulating
`Cluster::advance()` from `GlyphRun::offset()`. That works for our single-scalar,
one-glyph-per-cluster case, but it duplicates the offset math Parley already does inside
`GlyphRun::positioned_glyphs()` and is fragile for ligatures/RTL/multi-glyph clusters.

This change lets the renderer get **`(positioned Glyph, source char/cluster)` pairs directly**
so glyph→codepoint→custom-draw is a single clean iteration.

## Proposed change (additive, non-breaking)

In `parley/src/layout/line.rs`, add to `impl GlyphRun`:

```rust
/// Positioned glyphs paired with the source cluster they belong to, so callers can map
/// each glyph back to its source text (e.g. to substitute custom artwork for specific
/// codepoints).
pub fn positioned_glyphs_with_clusters(&'a self)
    -> impl Iterator<Item = (Glyph, Cluster<'a, B>)> + 'a + Clone
```

Implementation mirrors `positioned_glyphs()` but zips each glyph with the `Cluster` it came
from (the existing `visual_clusters().flat_map(|c| c.glyphs())` walk already knows the
owning cluster — surface it instead of discarding it). The caller then uses
`cluster.source_char()` / `cluster.text_range()` for the codepoint and `glyph.{x,y,advance}`
for placement.

Minimal variant (even smaller): expose only the char:
```rust
pub fn positioned_glyphs_with_source_char(&'a self)
    -> impl Iterator<Item = (Glyph, char)> + 'a + Clone
```

## How Phase Four uses it (render.rs)

```rust
if is_fake_font {
    for (g, cluster) in glyph_run.positioned_glyphs_with_clusters() {
        let cp = cluster.source_char() as u32;      // no manual x accumulation
        blit_apple_emoji(cp, g.x, glyph_run.baseline(), noto_ink_bounds(cp), font_size);
    }
    continue; // skip glifo for the empty fake glyphs
}
```

## Pros
- Removes the renderer's manual offset accumulation and cluster/glyph correlation.
- Correct for ligatures, multi-glyph clusters, and RTL out of the box.
- Purely additive; no existing signature changes; trivial to land upstream.

## Cons
- Still a Parley change, however small — Plan A needs none.
- Slightly widens the public API surface (`Cluster` in a return type; already public).

## Scope of change
- One new method (~10 lines) in `line.rs`. No data-structure changes. No behavior change to
  existing APIs. Docs + a unit test.

## Recommendation
Not needed for this phase's single-scalar scope; Plan A suffices. Prefer this **only** if we
want the example to be robust for general text (ligatures/RTL) or to model best practice for
downstream users doing glyph substitution.
