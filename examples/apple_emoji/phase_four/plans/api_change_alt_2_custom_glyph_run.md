# Phase Four — Parley API-change Alternative #2: first-class custom-draw / glyph-run interception

> Required by PLAN.md ("at least two possible API changes"). **Alternative to the
> recommended zero-change Plan A** (`phase_four_plan.md` §5). This is the larger, more
> general design — it removes the renderer's reliance on font-blob-identity as an
> out-of-band signal and makes "this run is drawn by the application" a supported concept.

## Motivation

Plan A detects the fake font by comparing `run.font().data.id()` against a blob id the
example stashed. That works but is implicit: nothing in Parley's API says "these glyphs are
placeholders to be custom-drawn." A first-class hook makes the intent explicit and robust to
font-registration internals (R5 in the main plan), and generalises to any
application-provided glyph artwork (icons, custom emoji, math atoms).

Two sub-designs are offered; **2A is the lighter, recommended form of this alternative.**

## Design 2A — brush-carried "custom run" marker + `Run::is_custom()`

Parley's `Brush` is user-defined (the example already uses `ColorBrush`). Extend the
example's brush with a flag, and add a tiny Parley affordance so the flag can be read off a
run without re-reading styles.

- Application side (no Parley change strictly needed): put `custom_emoji: bool` on the
  brush, push it as a style span over emoji ranges. In `render.rs`, read
  `glyph_run.style().brush.custom_emoji`. **This actually needs zero Parley change** — it is
  a variant of Plan A that keys on brush instead of font identity, arguably cleaner. Include
  it here as the "no-API-change but not-font-identity" option.

- Optional Parley convenience: add `GlyphRun::style()` already exists, so even the read is
  free. So 2A is really "Plan A, keyed on brush." Documented as a fallback if blob-id
  identity proves unstable.

## Design 2B — `PositionedLayoutItem::CustomGlyphRun` + registered drawer (the real API change)

Introduce an explicit interception point in layout iteration.

### API sketch (`parley/src/layout/`)
```rust
/// Opt-in marker attached to a run (via a new StyleProperty or via FontData tagging)
/// telling Parley the consumer will draw these glyphs itself.
pub enum PositionedLayoutItem<'a, B: Brush> {
    GlyphRun(GlyphRun<'a, B>),
    InlineBox(PositionedInlineBox),
    CustomGlyphRun(GlyphRun<'a, B>),   // NEW: same data, different intent
}
```
A run is surfaced as `CustomGlyphRun` when its font (or a new `StyleProperty::CustomDraw(tag)`)
was registered as custom. The renderer matches the new arm and draws its own artwork,
getting the same `GlyphRun` (font, positioned glyphs, baseline, clusters) it would otherwise.

Alternative surface: a callback registered on the layout/render path,
`fn(run: &GlyphRun) -> bool` returning "handled", so no enum change is needed — but a
callback fits Parley's non-rendering role poorly (Parley does not own a render loop), so the
enum-variant form is cleaner.

### How Phase Four uses it
```rust
match item {
    PositionedLayoutItem::CustomGlyphRun(run) => draw_apple_emoji(run, ...),
    PositionedLayoutItem::GlyphRun(run) => render_normally(run, ...),
    PositionedLayoutItem::InlineBox(_) => {}
}
```

## Pros
- Explicit, self-documenting; no reliance on blob-id identity internals (R5 dissolved).
- Generalises to any app-drawn glyph content (icon fonts, custom emoji, math).
- 2A (brush-keyed) needs zero Parley change and is a clean drop-in.

## Cons
- 2B changes a core public enum (`PositionedLayoutItem`) — every `match` on it must add an
  arm ⇒ **breaking**, larger review surface, and needs a tagging mechanism
  (`StyleProperty::CustomDraw` or font tagging) plumbed through style resolution. This is
  substantially more than the phase needs.
- Adds a concept ("custom runs") Parley must maintain long-term.

## Scope of change
- 2A: zero Parley LOC (example brush field only).
- 2B: new enum variant + new `StyleProperty`/font tag + threading through `line.rs`
  item iteration and style resolution; docs; tests. Non-trivial, breaking.

## Recommendation
Do **not** implement 2B for this phase — it is disproportionate. If a non-font-identity
signal is wanted, use **2A (brush-keyed)**, which is zero-change and robust. Overall still
prefer Plan A (font-identity), with 2A as the fallback if `Blob::id()` stability through
fontique registration disappoints.
