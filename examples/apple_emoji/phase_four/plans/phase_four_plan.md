# Phase Four Implementation Plan — Apple emoji as real Parley glyphs, baseline-correct, via a fake metrics-only Noto font

## 1. Goal (from PLAN.md §Phase Four)

Match **baselines** as well as advances. Instead of phase_two/three's inline boxes, the
emoji now flow through Parley as **real glyphs** selected/shaped from a tiny **fake font**
that carries **Noto Color Emoji's metrics** (cmap coverage, per-glyph advances, upem,
ascent/descent/line-gap, vertical metrics) but **no glyph image data**. At render time each
fake-Noto glyph is replaced by the equivalent **scaled Apple system emoji**, positioned
baseline-correctly.

Constraints carried from earlier phases:
- Never embed or serve the Apple emoji font.
- Never ship the full 25 MB Noto font in the app path (only the tiny fake font, < 150 KiB).
- Minimise Parley changes; **plan A below needs zero** (see §5 and the two required
  API-change alternatives in `api_change_alt_1_glyphrun_source.md` and
  `api_change_alt_2_custom_glyph_run.md`).
- Nothing committed; work lives in `examples/apple_emoji/phase_four/`.

Work location: `examples/apple_emoji/phase_four/`, copied+extended from `phase_three`
(do **not** edit phases one–three; copy code).

## 2. Verified facts (this repo, probed with fonttools + source reading)

### 2.1 NotoColorEmoji-Regular.ttf (`parley_dev/assets/fonts/noto_color_emoji/`)
- **upem = 1024**, `numGlyphs = 41863`, cmap entries = **1499** (`0x20`..`0xFE837`).
- **hhea ascent = 950, descent = -250, lineGap = 0**; OS/2 winAscent=950, winDescent=250;
  sTypoAscender=950, sTypoDescender=-250, sTypoLineGap=0. Vertical metrics present
  (`vhea`/`vmtx`) but not needed for horizontal LTR layout.
- Advances essentially binary: 1461 cmapped codepoints have advance **1275**, 38 have 0
  (VS/ZWJ/combining). Our two example emoji `😀` U+1F600 and `🎉` U+1F389 → single glyphs,
  advance **1275** ⇒ `1275/1024 × 48 = 59.766 px` @48.
- This build is **COLR/CPAL + SVG** (vector), with `GSUB`, empty base `glyf`/`loca`. Base
  outlines are empty; color comes from COLR layers. (Relevant only as a warning that ink
  bounds must come from COLR, not `glyf`.)

### 2.2 Parley render surface — what the renderer already sees per glyph run (source-read)
`parley/src/layout/{run.rs,line.rs,glyph.rs,cluster.rs}`:
- `GlyphRun::run() -> &Run`, `run.font() -> &FontData`, `run.font_size()`,
  `run.normalized_coords()`.
- **`FontData` (from `linebender_resource_handle`) has public `data: Blob<u8>` and
  `index: u32`; `Blob::id() -> u64` is a stable per-blob unique id.** So the renderer can
  test run-font identity: `run.font().data.id() == fake_font_blob_id`.
- `GlyphRun::positioned_glyphs() -> impl Iterator<Item = Glyph>` with `Glyph { id, x, y,
  advance }` (x/y already include `offset`/`baseline`). `GlyphRun::baseline()`,
  `offset()`, `advance()`.
- **`Cluster` exposes `source_char() -> char`, `text_range()`, `is_emoji() -> bool`,
  `advance()`, `glyphs()`.** So the codepoint behind each emoji glyph is recoverable at
  render time with **no** extra table and no Parley change.

**Conclusion:** the renderer already has (font identity, glyph id, glyph position, baseline,
source codepoint). Zero Parley API changes are required to intercept and custom-draw the
fake-Noto glyphs. This is Plan A and the recommendation.

### 2.3 Available crates (in cargo registry cache / workspace)
- `write-fonts 0.48.1` (fontations font *writer*) — present in registry cache.
- `skrifa 0.43.2`, `read-fonts 0.40.0` — workspace deps (readers).
- `subsetter 0.2.4`, `ttf-parser`, `owned_ttf_parser` — present.
- Python `fonttools 4.62.1` available (used for probing/validation cross-checks only).

## 3. Fake font design ("metrics-only Noto")

### 3.1 Required tables (everything the shaper/metrics need, nothing drawable)
`head, hhea, maxp, hmtx, cmap, name, OS/2, post`, plus **empty** `glyf` + `loca` (all
glyphs zero-contour). No `COLR/CPAL/SVG/CBDT/CBLC` (that's the image data we must omit).
Values copied verbatim from Noto:
- `head.unitsPerEm = 1024`, matching flags/bbox (bbox can be 0).
- `hhea.ascender=950, descender=-250, lineGap=0`, `numberOfHMetrics` per hmtx.
- `OS/2` win/typo ascender/descender/lineGap as §2.1 (these drive Parley line height &
  baseline — must match exactly).
- `hmtx`: advance width per glyph = Noto's advance for that glyph.
- `cmap`: the same codepoint→gid mapping as Noto for the 1499 covered codepoints (format 12).
- `maxp.numGlyphs` = number of glyphs we keep (see §3.2).
- `post` format 3 (no names), `name` minimal (family = e.g. `"FakeNotoEmoji"`).

### 3.2 Glyph-count reduction (keep < 150 KiB, keep metrics identical)
We do **not** need 41863 glyphs. We only need glyphs for the **cmapped codepoints** the
example (and validation corpus) uses, remapped to a compact gid space:
- Build a new gid space: gid 0 = .notdef, then one gid per kept codepoint.
- Rewrite `cmap` so `cp → new_gid`, and `hmtx[new_gid] = Noto advance(cp)`.
- Keep a generator-side table `new_gid → cp` (and `cp → advance`) for the renderer's
  reverse lookup fallback (the renderer prefers `Cluster::source_char()`; the table is a
  belt-and-braces cross-check / for non-`source_char` paths).
- Because advances and cmap gids-as-seen-by-shaping are metric-equivalent, **Parley/harfrust
  shaping of single-scalar emoji yields identical cluster/glyph advances to real Noto**
  (same cmap target-advance and upem; outlines irrelevant to advance).
- Even keeping **all 1499** cmapped codepoints: ~1499 glyphs × (empty glyf 0 bytes + loca
  4 bytes + hmtx 4 bytes) + cmap format-12 (~12 bytes/group, but ranges are sparse so
  ~1499 groups ≈ 18 KB) + fixed tables ⇒ well under 150 KiB (estimate 30–50 KiB). Keeping
  only the example's codepoints is far smaller. **Recommendation: keep all 1499** for
  generality; it is still tiny.

### 3.3 GSUB / ZWJ caveat
Noto uses `GSUB` to ligate ZWJ/flag/keycap sequences into single advance-1275 glyphs. Our
fake font omits GSUB, so **multi-scalar emoji would shape as multiple glyphs** and mis-sum
advances. The example uses **single-scalar** emoji only (both 1275), so this is out of
scope — documented as **R3**. (If ever needed: copy Noto's GSUB and the participating gids;
adds size + complexity. Not for this phase.)

### 3.4 Builder: how to produce the fake font (two options, recommend A)

**Option A (recommended) — `write-fonts` generator crate.**
A native `publish = false` crate `examples/apple_emoji/phase_four/fake_font_gen/`:
1. Read Noto with `skrifa`/`read-fonts`: `head.units_per_em`, `hhea`, `OS/2`, `charmap`,
   `hmtx`.
2. Collect `(cp, advance)` for every cmapped cp (mirrors phase_three's advance gen).
3. Assign compact gids, build `write-fonts` table builders (`Head, Hhea, Maxp, Hmtx, Cmap
   (format 12), Name, Os2, Post`, empty `Glyf`+`Loca`), compile to bytes.
4. Emit two artifacts (uncommitted):
   - `src/fake_noto.ttf` (the < 150 KiB font, `include_bytes!`'d by the app), **or** an
     embedded `&[u8]` in a generated `.rs`.
   - `src/fake_noto_meta.rs`: `FAKE_UPEM`, `GID_TO_CP`/`CP_TO_ADV` slices (reverse lookup).
5. Run once locally: `cargo run -p fake_font_gen`.

Rationale: keeps native-only deps out of the wasm crate (same isolation reason phase_three
used a separate `noto_advance_gen`); `write-fonts` is the fontations-native, well-typed
writer and is already in the registry cache.

**Option B (fallback) — Python fonttools script.** A `build_fake_font.py` using
`fontTools.fontBuilder.FontBuilder` (setupGlyf empty, setupCharacterMap, setupHorizontal
Metrics, setupHead/Hhea/OS2/Name/Post). Also fine and very concise, but adds a Python build
step outside the Rust toolchain and is less reproducible in CI. Use only if `write-fonts`
proves awkward.

**Do not** hand-roll raw TTF bytes (error-prone checksums/offsets) when `write-fonts`
exists.

### 3.5 Validate the fake font matches Noto *before* proceeding (PLAN.md requirement)
A native test/bin `fake_font_gen` (or a `#[test]`) that lays out the same strings with (a)
the fake font and (b) real Noto, both via Parley, and asserts identical:
- `layout.width()`, per-line advances, `line.metrics()` ascent/descent/line-height,
  `GlyphRun::baseline()`, and per-emoji `Cluster::advance()`.
This is the crispest, fully-native "same metrics" proof and must pass before the browser
work is meaningful. (Real Noto may be loaded natively here — it is validation-only, not the
app path.)

## 4. Layout changes vs phase_three

Phase_three stripped emoji and inserted inline boxes. Phase_four **keeps the emoji in the
text** and lets Parley shape them from the fake font:
- Register Arimo (as before) **and** the fake font blob; capture the fake font's
  `Blob::id()` (store it for the renderer identity check).
- Style: default family stack `["Arimo", "FakeNotoEmoji"]`. Arimo lacks emoji ⇒ Parley
  fallback selects FakeNotoEmoji for emoji codepoints. **Deterministic alternative**
  (recommended for robustness): explicitly `push` a `FontFamily::named("FakeNotoEmoji")`
  style span over each emoji `char` range so selection cannot pick a system emoji font.
- No inline boxes at all now. Segmentation (`is_emoji_char`) is reused only to place the
  explicit family spans (single-scalar scope as before).

## 5. Rendering interception (Plan A — zero Parley changes; **recommended**)

In `render.rs`, in the `PositionedLayoutItem::GlyphRun` arm:
1. Compute `is_fake = glyph_run.run().font().data.id() == fake_font_blob_id`.
2. If **not** fake: render normally via `GlyphRunBuilder` (unchanged from phase_three).
3. If fake: **skip glifo entirely** (its glyphs are empty anyway) and custom-draw:
   - Walk the run's clusters in visual order, tracking `x` from `glyph_run.offset()`,
     incrementing by `cluster.advance()`; `baseline = glyph_run.baseline()`.
     (Each single-scalar emoji = one cluster = one glyph.)
   - `cp = cluster.source_char() as u32` → look up the Apple `EmojiCapture` (captured as in
     phases one–three) and the Noto advance `noto_px` (= glyph advance = `cluster.advance()`).
   - Uniform `scale = noto_px / apple_advance_px` (as phase_three §5.2).
   - **Baseline-correct blit** (the new part, §6): place scaled Apple ink relative to
     `baseline` using Noto's emoji ink-box, not phase_two's ad-hoc baseline.
4. Because Parley computed line height/baseline from the fake font's Noto-matching
   ascent/descent, the baseline the renderer receives is already the Noto baseline.

The blit is applied to the final pixmap after `render()` (same staging as phase_three; the
fake glyph run contributes nothing to the vello_cpu pass, so ordering is safe). Optionally
just collect `(cp, x, baseline, scale)` records during iteration and blit them after
`render()`, exactly mirroring phase_three's inline-box loop.

**Font identity detail:** the fake font blob is created once with
`Blob::new(Arc::new(bytes))`; its `.id()` is stored. Registering via
`font_cx.collection.register_fonts(blob.clone(), None)` keeps the same id, so
`run.font().data.id()` matches. (Verify fontique doesn't re-wrap the blob with a new id; if
it does, fall back to comparing `run.font().data.data()` pointer/length or the family name
via `run.font_attrs()` / a family lookup — noted as **R5**.)

## 6. Baseline-correct vertical placement (the phase-four deliverable)

Model: Noto positions an emoji glyph within its em relative to the baseline. We must place
the scaled Apple ink so its ink occupies the **same vertical band relative to `baseline`**
that Noto's emoji would.

Approach (measurable, data-driven):
1. At generation time, compute each Noto emoji's **ink bbox in font units** (yTop, yBottom
   relative to baseline). Base `glyf` is empty here, so derive bounds from **COLR layer
   bounds** via skrifa's color/outline glyph bounds (`skrifa::color`/outline `bounds` with
   the emoji's layers) — or, pragmatically, use a single representative band since Noto
   emoji are near-uniformly designed to fill roughly `y ∈ [-p·upem, (1-q)·upem]`. Store the
   per-glyph (or global) `noto_ink_top_units`, `noto_ink_bottom_units`.
2. Convert to px at font size: `noto_top_px = noto_ink_top_units/1024 * font_size` above
   baseline; similarly bottom.
3. When blitting, map the Apple capture's ink so that:
   - the Apple ink's **top** aligns to `baseline - noto_top_px`,
   - and it is uniformly scaled by `scale` (advance-matching) — then **fit vertically** to
     `[baseline - noto_top_px, baseline + noto_bottom_px]`. Since we scale uniformly for
     advance, vertical fit is a *check*, not a second scale; residual mismatch is the
     Apple-vs-Noto design-height difference and is reported.
4. Practical fallback if COLR bounds are fiddly: derive the single vertical offset
   empirically by matching the browser real-Noto reference screenshot (§7) once, bake it as
   `NOTO_EMOJI_BASELINE_RATIO`, and document it. This keeps the phase shippable.

Deliverable acceptance for baselines is defined in §8 against the browser reference.

## 7. Validation

### 7.1 Native metric parity (primary, crispest) — §3.5
Fake-vs-real-Noto Parley layout diff: identical width, line metrics, baselines, per-emoji
advance. Must pass before browser steps. Fully native, no Apple font.

### 7.2 Browser visual + metric (ChromeDriver, as phases one–three)
- `trunk build`; serve `dist/`; drive Chrome for Testing (paths in PLAN.md; `--headless=new`,
  fall back to headed for system color emoji as prior phases found necessary); poll
  `#status==done`; scrape metrics JSON; `GET /screenshot` → `_artifacts/phase_four.png`.
- Real-Noto reference served **only** by the `validate.py` route (never in `dist/`), used by
  a hidden canvas/element to (a) `measureText` per-emoji + full line advance, and (b) render
  the string with real Noto for a **pixel** vertical-position reference.

### 7.3 Comparisons emitted to JSON + checked by validate.py
1. Parley `layout.width()` == real-Noto reference line width, ≤ 1 px (advance parity;
   already achieved in phase_three, must not regress).
2. Per-emoji `Cluster::advance()` == real-Noto `measureText(emoji)` (≤ 0.5 px).
3. **Baseline/vertical (new):** in the app screenshot vs the real-Noto reference screenshot,
   each emoji's ink **top and bottom edges** align within tolerance **≤ 2 px @48** (define
   via alpha-threshold ink bbox per emoji region). This is the phase-four acceptance metric.
4. Payload audit: grep `dist/` — no Apple font, no full Noto; only the < 150 KiB fake font.

## 8. Acceptance criteria
1. `write-fonts` generator produces a fake font **< 150 KiB** with upem 1024, ascent 950 /
   descent -250, and advances matching Noto for all covered codepoints.
2. Native parity test (§3.5/7.1): fake vs real Noto → identical width, line metrics,
   baselines, per-emoji advances (exact for advances; line metrics bit-identical).
3. `trunk build` on wasm32 succeeds; `cargo clippy` clean for app + generator.
4. Page reaches `done` in Chrome for Testing; emoji are drawn as **glyphs** selected from
   the fake font (not inline boxes) then replaced by scaled Apple pixels.
5. Advance parity ≤ 1 px (line) / ≤ 0.5 px (per emoji) vs real Noto.
6. **Baseline parity**: emoji ink top/bottom within ≤ 2 px of the real-Noto reference render.
7. Payload contains no Apple font and no full Noto (only the fake font).
8. Parley library **unchanged** under Plan A (only uncommitted example files + root
   `members` edits). Nothing committed.

## 9. File layout
```
examples/apple_emoji/phase_four/
├── Cargo.toml                 # wasm app (copied from phase_three; +fake font include)
├── index.html                 # + validation-only real-Noto @font-face / render ref
├── src/
│   ├── main.rs                # copied; register fake font, keep emoji in text
│   ├── capture.rs             # copied verbatim
│   ├── layout.rs              # emoji styled with FakeNotoEmoji family (no inline boxes)
│   ├── render.rs              # Plan A: font-identity interception + baseline blit
│   ├── fake_noto.ttf          # GENERATED (uncommitted), include_bytes!'d
│   └── fake_noto_meta.rs      # GENERATED: upem, gid→cp / cp→adv, ink bounds
├── fake_font_gen/             # native generator crate (uncommitted, publish=false)
│   ├── Cargo.toml             # deps: write-fonts, skrifa, read-fonts, parley (for parity test)
│   └── src/main.rs            # build font + parity test
├── validate.py                # copied; +real-Noto route, +baseline pixel compare
└── plans/
    ├── phase_four_plan.md               # this file
    ├── api_change_alt_1_glyphrun_source.md
    └── api_change_alt_2_custom_glyph_run.md
```
`.gitignore` (local): `/dist`, `src/fake_noto.ttf`, `src/fake_noto_meta.rs`.

Root `Cargo.toml` `members` (uncommitted): add `phase_four` and `phase_four/fake_font_gen`.

## 10. Risks / open questions
- **R1 — COLR ink bounds for baseline.** Deriving per-glyph Noto ink extent from COLR is
  the fiddliest part. Mitigation: global ratio + empirical browser tuning (§6.4). Baseline
  tolerance set at ≤ 2 px to absorb Apple-vs-Noto design differences.
- **R2 — Apple advance fallback (from phase-three validation).** Headless Chrome
  `measureText` returned advance == font size (48) for Apple emoji; `actualBoundingBox*`
  were reliable. So compute `apple_advance_px` for the `scale` from **ink bounds**
  (`actualBoundingBoxLeft/Right`), not `width`, or run headed. Carry phase-three's working
  capture approach.
- **R3 — Multi-scalar/ZWJ emoji** not supported (no GSUB in fake font). Single-scalar scope.
- **R4 — Fake-font selection.** Ensure fontique picks the fake font for emoji (explicit
  family span recommended over relying on fallback ordering).
- **R5 — Blob id stability through fontique.** If registration re-wraps the blob and changes
  `.id()`, identity check fails; fallback: match by family name via `run.font_attrs()` or by
  `data().as_ptr()/len()`. Verify at implementation.
- **R6 — 25 MB Noto served by harness** only during validation; keep off app path.

### Open questions needing team-lead ruling
- **Q1:** Fake-font builder — `write-fonts` Rust crate (recommended) vs Python fonttools?
- **Q2:** Keep all 1499 codepoints in the fake font (recommended, still tiny) vs only the
  example's?
- **Q3:** Baseline placement — data-driven COLR ink bounds vs single empirical ratio
  (recommended: ship empirical ratio, note COLR path as future work)?
- **Q4:** Confirm **Plan A (zero Parley changes)** is accepted as the implementation, with
  the two API-change plans provided only to satisfy PLAN.md's "two alternatives" requirement.
- **Q5:** Emoji font selection — explicit family span (recommended) vs fallback stack?
```
```
