# Phase Two Implementation Plan — Parley + Vello CPU text with Apple system-emoji inline boxes (wasm)

## 1. Overview / goal

Build a single Rust wasm app at `examples/apple_emoji/phase_two` that runs in the
browser and:

1. **Measures + captures** each emoji in an example string using the Phase-One canvas
   technique (`ctx.measureText` for advance + bounding boxes, `fillText` + `getImageData`
   for RGBA pixels), at the **text font size**.
2. **Builds a Parley layout** of the example text using the in-repo **Arimo** variable
   font, where every emoji is replaced by a Parley **inline box** whose `width` equals the
   Apple-system-font advance measured in step 1.
3. **Renders** the layout with **`vello_cpu`** (via the `glifo` `GlyphRunBuilder` path,
   copied from `examples/vello_cpu_render`) to an RGBA `Pixmap`, then **blits the captured
   emoji pixels** into each `PositionedInlineBox` rect, aligned so the emoji's alphabetic
   baseline matches the Parley text baseline.
4. **Presents** the Parley+Vello result in a canvas via `putImageData`, directly beside a
   **browser-native rendering** of the same string (Arimo served from the repo via
   `@font-face`, emoji from the system fallback) for visual comparison.

Validation: a ChromeDriver Python harness (reusing Phase-One's `validate.py`) loads the
page, scrapes per-emoji metrics + alignment data as JSON, and captures a screenshot
showing the native reference above the Parley+Vello output.

**Constraints honored:** no Apple emoji font embedded/served (only Arimo, which is in-repo
and allowed); no Parley library changes; nothing committed.

## 2. Repository context (verified facts)

- **Inline box vertical semantics — the crux (verified in code):**
  - `parley/src/layout/line_break.rs:929-932`: for an in-flow inline box,
    `line.metrics.ascent = line.metrics.ascent.max(item.height)` with the comment
    *"Default vertical alignment is to align the bottom of boxes with the text baseline.
    This is equivalent to the entire height of the box being 'ascent'."*
  - `parley/src/layout/line.rs:285-289`: the `PositionedInlineBox` is produced with
    `x: inline_box.x` (left edge) and **`y: self.line.data.metrics.baseline - inline_box.height`**.
  - **Therefore: the inline box's bottom edge sits exactly on the text baseline**, and the
    box only reserves space *above* the baseline (it contributes `height` to the line
    ascent, nothing to descent). This is the anchor for all baseline math (§6).
- `PositionedInlineBox` fields (`parley/src/layout/line.rs:182-189`):
  `x, y, width, height: f32`, `id: u64`, `kind: InlineBoxKind`.
- `InlineBox` input struct (`parley/src/inline_box.rs:6-19`): `id: u64`,
  `kind: InlineBoxKind`, `index: usize` (byte offset into the text; must not split a code
  point), `width: f32`, `height: f32`. Added via `builder.push_inline_box(InlineBox{..})`
  (`parley/src/builder.rs:227`); boxes are sorted by `index` at build
  (`builder.rs:331`).
- **Iterating positioned items** (copied from `examples/vello_cpu_render/src/main.rs:103-166`):
  `for line in layout.lines() { for item in line.items() { match item {
  PositionedLayoutItem::GlyphRun(gr) => …, PositionedLayoutItem::InlineBox(ib) => … } } }`.
  The existing example already draws inline boxes as filled rects using
  `ib.x, ib.y, ib.width, ib.height` — we replace that fill with an emoji blit.
- **vello_cpu text rendering pipeline** (copy wholesale from `vello_cpu_render`):
  `GlyphRunBuilder::new(run.font().clone(), *renderer.transform()).font_size(...).hint(...)
  .normalized_coords(...).atlas_cache(true).build(glyphs, &mut glyph_caches, &mut image_cache)`
  then `renderer.set_paint(color); run_renderer.fill_glyphs(renderer);` and the
  `render()` helper (atlas replay → uploads → `register_image` → `render_to_pixmap`).
  We copy `prepare_rendering`, `reset_renderer`, `render`, `copy_pixmap_to_atlas`,
  `clear_pixmap_region` verbatim.
- **Font registration** (`fontique` `Collection::register_fonts`,
  `fontique/src/collection/mod.rs:502`): `font_cx.collection.register_fonts(Blob::new(Arc::new(bytes)), None)
  -> Vec<(FamilyId, Vec<FontInfo>)>`. Precedent: `examples/common/src/lib.rs:171-173`.
  We `include_bytes!` Arimo and register it; then style the text with the registered
  family name (read the family name from the returned `FontInfo`/`FamilyId`, or use
  `FontFamily::named("Arimo")`).
- **Pixmap** (`vello_cpu` 0.0.9): premultiplied RGBA8; `Pixmap::new(w,h)`,
  `data_as_u8_slice()`, `data_as_u8_slice_mut()`, `into_png()`, `width()/height()`.
  Direct buffer blit precedent: `copy_pixmap_to_atlas` in `vello_cpu_render`.
  **Note the format mismatch:** canvas `getImageData` is *straight* (non-premultiplied)
  RGBA; Pixmap is *premultiplied* RGBA. We premultiply during the blit (§6).
- Workspace uses an explicit `members` list (edition 2024, MSRV 1.88,
  `[lints] workspace = true`). `examples/apple_emoji/phase_one` is already a member.
- Arimo font present at
  `parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf`.
- Phase-One wasm/Trunk scaffolding (`index.html` with `<link data-trunk rel="rust"/>`,
  `wasm-bindgen`/`web-sys`/`js-sys`/`console_error_panic_hook`) and `validate.py`
  ChromeDriver harness are proven and reusable — copy, don't edit phase_one.

## 3. File layout

```
examples/apple_emoji/phase_two/
├── Cargo.toml
├── index.html                 # Trunk entry; serves Arimo via @font-face for the native ref
├── assets/
│   └── Arimo-VariableFont_wght.ttf   # copy of the in-repo Arimo (served for @font-face; allowed)
├── src/
│   ├── main.rs                # app entry + orchestration
│   ├── capture.rs             # Phase-One-derived emoji measure+capture (copied/adapted)
│   ├── layout.rs              # Parley layout build with inline boxes
│   └── render.rs              # vello_cpu render + emoji blit (copied from vello_cpu_render)
└── plans/
    └── phase_two_plan.md      # this document
```

Notes:
- Arimo for Parley is embedded via `include_bytes!` pointing at the repo path
  (`../../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf`) — no copy
  needed for the Rust side. For the **browser-native reference** we need Arimo served over
  HTTP for `@font-face`; Trunk copies files referenced from `index.html`. Either
  `<link data-trunk rel="copy-file" href="../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf"/>`
  (preferred — no duplicate file) or a local `assets/` copy. Decide at implementation time;
  copy-file avoids a duplicated binary. (Serving Arimo is explicitly allowed; only Apple
  emoji may not be served.)
- `/dist` added to a local `.gitignore` (courtesy; nothing committed).

## 4. Cargo.toml sketch

```toml
[package]
name = "apple_emoji_phase_two"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[dependencies]
# Parley + Vello CPU render stack (mirrors examples/vello_cpu_render)
parley = { workspace = true, default-features = true }
glifo = { workspace = true, default-features = false, features = ["std", "vello_cpu"] }
vello_cpu = { workspace = true, default-features = false, features = ["std"] }
peniko = { workspace = true }
skrifa = { workspace = true }

# wasm / browser glue (mirrors phase_one)
wasm-bindgen = "0.2.100"
wasm-bindgen-futures = "0.4.50"
js-sys = "0.3.77"
console_error_panic_hook = "0.1.7"

[dependencies.web-sys]
version = "0.3.77"
features = [
    "Window", "Document", "Element", "HtmlElement", "HtmlCanvasElement",
    "CanvasRenderingContext2d", "TextMetrics", "ImageData", "Node",
    "Navigator", "console", "FontFaceSet",
]

[lints]
workspace = true
```

- Drop the `png` feature (we present via canvas, not PNG files) — but the `validate.py`
  screenshot is taken by Chrome, not by us, so no PNG dependency is needed in-crate.
- **Workspace membership:** add `"examples/apple_emoji/phase_two"` to root `Cargo.toml`
  `members` (same approved pattern as phase_one; uncommitted, build-enabling only).

## 5. Architecture & data flow

All in one wasm app driven from `main()` → `spawn_local(run())` (Phase-One pattern).

```
run():
  1. await document.fonts.ready  (+ yield_now)          [capture.rs]
  2. EXAMPLE_TEXT = "Hi 😀 world 🎉!"  (const; simple)
  3. segment_emoji(EXAMPLE_TEXT) -> Vec<EmojiHit{ byte_index, grapheme: &str }]  [layout.rs]
  4. for each emoji: capture(grapheme, FONT_SIZE) -> EmojiCapture {              [capture.rs]
        advance_width, abb_left, abb_right, abb_ascent, abb_descent,
        capture_w, capture_h, origin_x, origin_y (baseline pos in buffer),
        pixels: Vec<u8> (straight RGBA)  }
     (capture at FONT_SIZE * SUPERSAMPLE for crispness; see §6 scaling)
  5. build_layout(text_without_emoji, captures) -> Layout<ColorBrush>           [layout.rs]
        - register Arimo via include_bytes! + register_fonts
        - push text runs; for each emoji, push_inline_box{ id: i, index,
          width: advance_width, height: box_height }  (see §6)
  6. render(layout) -> Pixmap  (glifo + vello_cpu, copied)                      [render.rs]
  7. for each PositionedLayoutItem::InlineBox: blit_emoji(pixmap, capture)      [render.rs]
        baseline-aligned src-over composite of captured pixels
  8. Pixmap -> ImageData -> putImageData onto #parley canvas                    [main.rs]
  9. write metrics JSON to #data-phase-two; set #status = "done"
 10. #native canvas/div renders EXAMPLE_TEXT natively (HTML, Arimo @font-face)  [index.html]
```

The **native reference** is pure HTML/CSS: a `<div>` (or a 2D canvas using
`fillText`) with `font-family: 'Arimo', ...; font-size: FONT_SIZE px`, containing
`EXAMPLE_TEXT`. Arimo is loaded via `@font-face` (served from repo); emoji resolve to the
system Apple Color Emoji via normal fallback. This is exactly what the browser would do,
so it is the ground truth for advance widths and baseline.

### Text/emoji segmentation (kept simple)

- Hardcode `EXAMPLE_TEXT`. Segment emoji with a small scan: iterate `char_indices()` and
  treat any `char` in emoji ranges (or use a tiny check: `c.is_ascii()` false + emoji
  property) as an emoji. To keep it robust for multi-scalar emoji (e.g. `🎉` is single, but
  ZWJ sequences aren't), **restrict the example string to single-`char` emoji** (`😀`,
  `🎉`, `👍`) — documented simplification. `EmojiHit.byte_index` is the byte offset in the
  *Parley text* (the text with emoji removed — see below).
- **Building the Parley text:** Parley must NOT shape the emoji chars (they'd hit a
  fallback font and produce tofu/COLR glyphs and add their own advance). So we build the
  Parley string with emoji **removed**, and record the byte offset in that stripped string
  where each inline box goes. Insert a marker? No — `push_inline_box{ index }` places the
  box at that byte offset without needing a character there. So: strip emoji, remember
  offsets, push boxes at those offsets. (The native reference uses the *full* string.)
  - Edge case: a space typically surrounds emoji in the example; keep one space on each
    side in both strings so spacing matches the native layout.

## 6. Exact baseline & advance math (the crux)

### Advance width
- `inline_box.width = capture.advance_width` = `measureText(emoji).width` measured with
  `ctx.font = "{FONT_SIZE}px 'Apple Color Emoji', ..."` at the **same FONT_SIZE** used for
  Parley text and the native reference. Same font size + same font ⇒ identical advance to
  the native layout. (Measure at FONT_SIZE, not the supersampled size; scale pixels
  separately.)

### Inline box height
- The box bottom sits on the baseline and the box reserves `height` **above** the baseline
  (§2). Set `inline_box.height = capture.abb_ascent` (actualBoundingBoxAscent — ink extent
  above baseline). This guarantees the line grows tall enough that the emoji's above-baseline
  ink is not clipped by too-small a line.
  - Alternative: use `fontBoundingBoxAscent` (font-level, slightly larger) for extra
    safety / to better mimic how the emoji font would size the line natively. **Recommend
    `fontBoundingBoxAscent`** so the line box matches the native emoji line contribution
    more closely. (Open question Q2 — pick after eyeballing.)
- The emoji's **below-baseline** ink (`abb_descent`) is NOT reserved by the box (Parley
  gives boxes no descent). It will draw below the baseline overlapping the text's descent
  region. For most emoji `abb_descent` is small; if the Arimo line descent is smaller, the
  emoji's bottom could slightly overhang the line — acceptable for this comparison, noted
  as a risk (R2).

### Baseline-aligned pixel blit
At draw time, for a `PositionedInlineBox ib` matched (by `id`) to its `EmojiCapture cap`:

- Parley text baseline (in layout coords, before the frame translate):
  `baseline_y = ib.y + ib.height`  (since `ib.y = baseline - height`).
- Add the frame translate (`Affine::translate(padding, padding)` in `reset_renderer`);
  we render into the pixmap in device pixels, so the destination baseline in pixmap pixels
  is `dev_baseline_y = padding + baseline_y` and `dev_left_x = padding + ib.x`.
  (Simplest: read `*renderer.transform()` translation, or just add the known padding.)
- In the **capture buffer**, the alphabetic baseline row is at `cap.origin_y`
  (`= PAD + cap.abb_ascent`, from the Phase-One draw origin), and the text origin column is
  at `cap.origin_x` (`= PAD + cap.abb_left`). Because we set `inline_box.width =
  advance_width` and the box left `ib.x` corresponds to the emoji's pen origin, the capture
  column `cap.origin_x` maps to `dev_left_x`.
- **Mapping (unscaled case, SUPERSAMPLE = 1):** for capture pixel `(sx, sy)`, destination
  pixmap pixel is:
  `dx = dev_left_x + (sx - cap.origin_x)`
  `dy = dev_baseline_y + (sy - cap.origin_y)`
  i.e. capture baseline row lands on the Parley baseline, capture origin column lands on
  the box left. Blit every non-transparent capture pixel with src-over.
- **With supersampling (capture at `FONT_SIZE*S`):** capture metrics are all in the
  supersampled space. Downscale by `1/S` when mapping: destination
  `dx = dev_left_x + (sx - cap.origin_x)/S`, `dy = dev_baseline_y + (sy - cap.origin_y)/S`.
  Implement as a destination-space loop (for each dest pixel in the box's ink bbox, sample
  the source with nearest or bilinear) to avoid gaps — cleaner than a source-space scatter.
  Recommend rendering the whole app at the browser `devicePixelRatio` and capturing at that
  same scale so `S == dpr` and everything is crisp and 1:1 with the presentation canvas.

### Compositing (format)
- Canvas pixels are straight RGBA; Pixmap is premultiplied. For each source pixel
  `(r,g,b,a)` (0-255): premultiply `r' = r*a/255` etc., then src-over onto the
  premultiplied dest `d`:
  `out = src' + dest * (1 - a/255)` per channel including alpha. Write back into
  `pixmap.data_as_u8_slice_mut()` at `((dy*width + dx)*4)`. Bounds-check dst.

## 7. Parley/vello_cpu API usage sketch (verified names)

```rust
// --- layout.rs ---
use parley::{FontContext, LayoutContext, InlineBox, InlineBoxKind, StyleProperty,
             FontFamily, Alignment, AlignmentOptions, LineHeight};
use parley::fontique::Blob;
use std::sync::Arc;

const ARIMO: &[u8] = include_bytes!(
  concat!(env!("CARGO_MANIFEST_DIR"),
    "/../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf"));

let mut font_cx = FontContext::new();
let families = font_cx.collection.register_fonts(Blob::new(Arc::new(ARIMO.to_vec())), None);
// family name for styling: read from `families[0]` FontInfo, or FontFamily::named("Arimo").

let mut layout_cx = LayoutContext::<ColorBrush>::new();
let mut builder = layout_cx.ranged_builder(&mut font_cx, &parley_text, display_scale, quantize);
builder.push_default(StyleProperty::Brush(ColorBrush{ color: Color::BLACK }));
builder.push_default(StyleProperty::FontStack(/* Arimo family */));
builder.push_default(StyleProperty::FontSize(FONT_SIZE_f32));
builder.push_default(LineHeight::FontSizeRelative(1.3));
for (i, (hit, cap)) in emojis.iter().enumerate() {
    builder.push_inline_box(InlineBox {
        id: i as u64,
        kind: InlineBoxKind::InFlow,
        index: hit.byte_index,               // byte offset in stripped parley_text
        width: cap.advance_width as f32,
        height: cap.box_height as f32,        // fontBoundingBoxAscent (see §6)
    });
}
let mut layout = builder.build(&parley_text);
layout.break_all_lines(max_advance);         // None => single line, or width for wrap
layout.align(Alignment::Start, AlignmentOptions::default());

// --- render.rs (copied from examples/vello_cpu_render/src/main.rs) ---
// prepare_rendering, reset_renderer, render, copy_pixmap_to_atlas, clear_pixmap_region.
for line in layout.lines() {
  for item in line.items() {
    match item {
      PositionedLayoutItem::GlyphRun(glyph_run) => { /* GlyphRunBuilder … fill_glyphs */ }
      PositionedLayoutItem::InlineBox(ib) => {
        // DO NOT fill_rect (that was the placeholder). Record (ib.id, ib.x, ib.y,
        // ib.width, ib.height) for the post-render emoji blit pass.
        boxes.push(ib);
      }
    }
  }
}
let mut pixmap = render(&mut renderer, &mut glyph_caches, &mut image_cache, w, h, &mut glyph_renderer);
for ib in boxes { blit_emoji(&mut pixmap, &captures[ib.id as usize], &ib, padding); }
```

- `ColorBrush`, `prepare_rendering`, `render`, etc. are copied from `vello_cpu_render`
  (and its `parley_examples_common::ColorBrush`) — but to avoid a dep on `parley_examples_common`
  and its native `std::fs`/output-dir helpers (unfriendly to wasm), **copy the small pieces
  we need directly into this crate** rather than depending on `examples/common`.
- Canvas presentation: `ImageData::new_with_u8_clamped_array_and_sh(Clamped(&straight_rgba),
  w, h)` then `put_image_data`. **Pixmap is premultiplied → un-premultiply to straight RGBA
  before building the presentation `ImageData`** (canvas expects straight alpha). Or present
  on an opaque white background (fill pixmap white first, then all alpha=255) so premult ==
  straight and no conversion needed — **recommend opaque white background** (matches the
  native reference's white page and sidesteps the conversion). Then just copy bytes.

## 8. index.html sketch

```html
<!DOCTYPE html><html lang="en"><head><meta charset="utf-8"/>
<title>Apple emoji — Phase Two: Parley+Vello vs native</title>
<link data-trunk rel="rust" />
<link data-trunk rel="copy-file"
      href="../../../parley_dev/assets/fonts/arimo_fonts/Arimo-VariableFont_wght.ttf"/>
<style>
  @font-face { font-family: 'Arimo';
    src: url('Arimo-VariableFont_wght.ttf') format('truetype'); }
  body { margin: 1rem; background: #fff; }
  #native { font-family: 'Arimo', sans-serif; font-size: 48px; white-space: pre;
            background:#fff; }
  canvas { display:block; image-rendering: pixelated; background:#fff; }
  .row { margin: 8px 0; }
  .label { font: 12px ui-monospace, monospace; color:#555; }
</style></head><body>
  <div id="status">Starting…</div>
  <div class="row"><div class="label">Browser-native (Arimo + system emoji):</div>
    <div id="native">Hi 😀 world 🎉!</div></div>
  <div class="row"><div class="label">Parley + Vello CPU (emoji = inline boxes):</div>
    <canvas id="parley"></canvas></div>
  <div id="data-phase-two" hidden></div>
</body></html>
```

- Wait for `document.fonts.ready` so Arimo is loaded before both the native div paints and
  before we could (optionally) also measure Arimo — the emoji capture uses Apple Color
  Emoji, not Arimo, but gating on fonts.ready is cheap insurance (Phase-One pattern).
- Keep the native `#native` and the `#parley` canvas at the **same font size and left
  origin** so the screenshot overlays cleanly. Consider absolute-positioning them at the
  same x so a validator can visually compare advance widths / baselines column by column.

## 9. Validation procedure

Reuse Phase-One's `validate.py` (copy to `phase_two/validate.py`, adjust ids/paths):

1. `trunk build` in `phase_two`; serve `dist/` over HTTP (stdlib server, as phase_one).
2. Launch Chrome for Testing via ChromeDriver (paths from `PLAN.md`); try `--headless=new`
   first, fall back to headed if system color emoji don't render headless (Phase-One R1).
3. Navigate; poll `#status` until `"done"`.
4. Scrape `#data-phase-two` JSON: per emoji `{ id, char, advance_width, abb_ascent,
   abb_descent, box_x, box_y, box_width, box_height, parley_baseline_y }`, plus overall
   layout width/height and FONT_SIZE.
5. **Cross-measure the native reference from the same page** via `execute/sync`: for each
   emoji, run canvas `measureText` in the page context on the native font stack at
   FONT_SIZE and return advances; compare to the inline-box widths (they share the same
   measurement, so this is a consistency check).
6. `GET /screenshot` → PNG → `_artifacts/phase_two.png`.

### Acceptance criteria

1. `trunk build` succeeds on `wasm32-unknown-unknown`; `cargo clippy` clean under
   workspace lints.
2. Page reaches `"done"` within timeout in Chrome for Testing.
3. **Advance parity:** for each emoji, `inline_box.width == measureText(emoji).width` at
   FONT_SIZE (exactly equal — same source). The total Parley layout line advance matches
   the native line advance within **≤ 1px per emoji** (allowing font-hinting/rounding of
   the surrounding Arimo runs).
4. **Baseline alignment:** in the screenshot, the emoji ink baseline aligns with the Arimo
   text baseline within **≤ 2px** (measure by comparing the bottom of a baseline-resting
   glyph like "H" to the emoji). Automated check: compare `parley_baseline_y` to the
   detected native baseline row.
5. **Ink-column comparison:** overlay/diff the ink columns of native vs Parley output; the
   emoji horizontal extents should coincide within a few px (advance is exact; ink offset
   depends on `abb_left`, which we honor in the blit).
6. **Visual:** screenshot shows native reference and Parley+Vello output with visibly
   matching text, matching emoji sizes/positions, correct colors (real Apple emoji).
7. **No Apple font served/embedded:** grep crate + `dist/` — only Arimo (`.ttf`) present.
8. No Parley library files changed; only the uncommitted root `members` edit + new files
   under `examples/apple_emoji/phase_two/`. Nothing committed.

## 10. Risks / open questions

- **R1 — vello_cpu/glifo on `wasm32-unknown-unknown` (biggest build risk).** The
  `vello_cpu_render` example is native-only; `vello_cpu` 0.0.9 + `glifo` may use SIMD /
  features that need care on wasm. Mitigation: build early with `trunk build` before wiring
  everything; if a feature breaks, try toggling `vello_cpu`/`glifo` features (we already
  drop `png`). **Flag to team-lead if it does not compile to wasm** — this is the single
  largest feasibility unknown and worth verifying first.
- **R2 — emoji below-baseline overhang.** Parley boxes reserve no descent, so emoji ink
  below the baseline (`abb_descent`) draws into the line's descent region; if Arimo's
  descent is smaller it may slightly overhang the line box. Cosmetic for this comparison;
  document. (Could inflate line height via `LineHeight` if needed.)
- **R3 — headless system color emoji.** Same as Phase-One R1; fall back to headed
  ChromeDriver if needed.
- **R4 — premultiplied vs straight alpha.** Handled by rendering on an opaque white
  background (§7) so no conversion is needed; if we ever need transparency, add explicit
  (un)premultiply.
- **R5 — devicePixelRatio / crispness.** Capture + render at `dpr` for crisp emoji and 1:1
  presentation; keep the native `#native` at CSS px. Document the scale used.
- **R6 — family name for Arimo.** `register_fonts` returns `FamilyId`s; the safest style is
  to resolve the family name from the returned `FontInfo` rather than assuming
  `"Arimo"`. Confirm the parsed family name at implementation time.

### Open questions needing a team-lead ruling
- **Q1:** OK to add `"examples/apple_emoji/phase_two"` to root `Cargo.toml` `members`
  (uncommitted), same as phase_one? (Recommended.)
- **Q2:** Inline box `height` = `fontBoundingBoxAscent` (recommended, better matches native
  line sizing) vs `actualBoundingBoxAscent` (tighter). Either works for baseline; affects
  line height only.
- **Q3:** Serve Arimo via Trunk `copy-file` from the repo path (no duplicate) vs a copy in
  `assets/`. (copy-file recommended.)
- **Q4:** Example string — a short single-`char`-emoji string (`"Hi 😀 world 🎉!"`) is
  proposed to keep segmentation trivial. Confirm that's acceptable scope (ZWJ/multi-scalar
  emoji out of scope for Phase Two).
```

## 11. Summary

A single Trunk/wasm crate at `examples/apple_emoji/phase_two` that captures each emoji's
Apple-system advance + pixels via the Phase-One canvas technique, builds a Parley layout of
the example string using in-repo Arimo with each emoji replaced by an inline box sized to
that advance, renders the text with `vello_cpu` (copying the `vello_cpu_render` glifo
pipeline), then blits the captured emoji pixels into each `PositionedInlineBox` baseline-
aligned (box bottom == text baseline, verified in Parley source), and presents the result
in a canvas beside a browser-native Arimo+system-emoji rendering. Validated by a
ChromeDriver harness (copied from Phase-One) that scrapes per-emoji metrics and screenshots
the native-vs-Parley comparison, with advance-parity and baseline-alignment acceptance
criteria. No Apple font served/embedded, no Parley changes, nothing committed.
