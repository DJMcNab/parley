# Kashida justification in Parley — feasibility & implementation plan

**Status:** investigation + experiments complete. Verdict below.
**Date:** 2026-07-04

Kashida justification is Arabic (and Syriac/Mongolian/N'Ko) justification that
adds width *inside* words by elongating the cursive joins (classically by
inserting U+0640 TATWEEL / "kashida", or by stretching the baseline), instead of
only widening the spaces between words. This document assesses how to add it to
Parley on top of HarfRust (shaping) and Skrifa (metrics/outlines), names the
concrete code that would change, and records the experiments that back the
conclusions.

---

## TL;DR / verdict

- **Feasible today, without re-shaping, for the common case.** HarfRust 0.10
  already exposes exactly the primitive needed: the
  `SAFE_TO_INSERT_TATWEEL` glyph flag (opt-in via
  `BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL`). Our experiment
  (`experiments/harfrust_probe`) confirms that inserting the Skrifa-resolved
  tatweel glyph at a flagged position reproduces the fully re-shaped result
  exactly for Noto Kufi Arabic — base glyph ids and advances are unchanged;
  elongation is linear in the number of tatweels.
- **What's missing in Parley is plumbing, not capability.** Parley currently
  (a) never sets the produce-tatweel buffer flag, (b) discards HarfRust glyph
  flags entirely (they're read in `data.rs` but not stored), and (c) has no
  per-cluster field to carry "kashida opportunity" or "elongation applied", and
  no path to surface synthesized glyphs from `GlyphRun`.
- **What's missing upstream:** HarfRust has **no** `hb_shape_justify` and does
  **not** consume the OpenType `JSTF` table. HarfBuzz's own `hb_shape_justify`
  (7.1.0) is experimental, variable-axis-based, and not ported. So Parley must
  own the justification policy (where + how much) itself; HarfRust only tells us
  *where insertion is safe*.
- **Rough effort:** a solid v1 (tatweel-glyph insertion, distribute like the
  existing space justification, LTR/RTL, opt-in mode) is **medium** — on the
  order of the existing justification code plus cluster-flag plumbing and a
  render-side glyph synthesis path. High-quality calligraphic/variable-font
  kashida is a larger, font-dependent follow-up.

---

## 1. Parley's current justification pipeline

### Where it lives
- `parley/src/layout/alignment.rs` — `Alignment` enum (incl. `Justify`) and
  `align()` / `unjustify()` / `align_impl::<UNDO_JUSTIFICATION>()`.
- `parley/src/layout/line_break.rs` — line breaking; counts `num_spaces` per
  line (the justification opportunities).
- `parley/src/layout/data.rs` — `ClusterData` (`advance`, `flags`, `info`),
  `LineData` (`num_spaces`, `break_reason`, metrics), `RunData`,
  `process_clusters()` (turns HarfRust output into clusters/glyphs).
- `parley/src/layout/data.rs::ClusterInfo::whitespace()` +
  `analysis/cluster.rs::Whitespace::is_space_or_nbsp()` — classify a cluster as
  a justifiable space.

### How `Alignment::Justify` works today
1. **Line breaking** (`line_break.rs`) counts, per line, the number of
   space/nbsp clusters into `LineData::num_spaces` (see the `is_space_or_nbsp()`
   branches around lines 563–632 and 789–805; `num_spaces` is finalized in
   `commit_line` around line 1259). Trailing whitespace at the break is excluded.
2. **Alignment** (`align_impl`, alignment.rs:95–193): for a `Justify` line that
   isn't the last line (`break_reason` not `None`/`Explicit`) and has
   `num_spaces > 0`, it computes
   `free_space = line_width - indent - advance + trailing_whitespace`, then
   `adjustment = free_space / num_spaces`, and walks the line's text runs
   (forward for LTR items, reversed for RTL) adding `adjustment` to the
   `advance` of each space/nbsp `ClusterData` until `num_spaces` have been
   adjusted.
3. **Undo**: justification mutates `ClusterData::advance` in place. Before
   re-line-breaking/re-aligning, `unjustify()` re-runs the same walk with the
   sign flipped (`UNDO_JUSTIFICATION = true`). `LayoutData::is_aligned_justified`
   tracks whether an undo is pending.

### Key observations for kashida
- **Per-cluster advance adjustment is already the mechanism.** Justification is
  "find opportunity clusters, add extra advance to each". Kashida is the same
  shape of operation with (a) a different set of opportunity clusters (cursive
  joins, not spaces) and (b) a rendering consequence (extra tatweel glyphs must
  be *drawn*, whereas widening a space just moves following glyphs).
- **`ClusterData::flags` is a `u16` with only 2 bits used**
  (`LIGATURE_START = 1`, `LIGATURE_COMPONENT = 2`; data.rs:38–39). There is room
  for a `KASHIDA_OPPORTUNITY` bit (and, if desired, a bit or small count for
  "elongation applied"). This is the natural home for per-cluster kashida data.
- **`LineData::num_spaces`** is the template for a parallel
  `num_kashida_opportunities` (or a priority-ranked count) computed during line
  breaking.

---

## 2. Shaping with HarfRust

### Which shaper
HarfRust (`harfrust = 0.10.0`, workspace dep; used in `parley/src/shape/mod.rs`
and `data.rs`). Swash is only a dev/example dependency. Skrifa provides metrics
(`data.rs` uses `skrifa::metrics::Metrics`), and would provide the tatweel
glyph's advance/outline.

### Can kashida be done WITHOUT re-shaping? — **Yes, for the common case.**
HarfRust exposes HarfBuzz's tatweel mechanism (all verified in the local 0.10.0
source and by experiment):

- `harfrust::BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL` (buffer input flag),
  set via the public `UnicodeBuffer::set_flags()`.
- `harfrust::GlyphInfo::flags() -> GlyphFlags` with
  `GlyphFlags::is_safe_to_insert_tatweel()` (and `is_unsafe_to_break()`,
  `is_unsafe_to_concat()`), `SAFE_TO_INSERT_TATWEEL = 0x4`.

Semantics (from HarfBuzz, corroborated by our probe): the flag marks the glyph
*before which*, in logical order, a U+0640 may be inserted **without changing the
shaping of the surrounding text**. It tells you *where* elongation is safe — not
*how much* to add nor whether it will look good.

**Experiment (`experiments/harfrust_probe`, full output in its `RESULTS.md`):**
shaping `كتاب` vs. re-shaping `كـتاب` / `كــتاب` (explicit tatweels inserted at
a flagged join) yields **identical base-letter glyph ids and advances**; the
only change is 1–2 extra tatweel glyphs (Skrifa: gid 114, advance 171 upem).
Elongation is exactly `n × tatweel_advance`. So:

- **(a) Post-shaping tatweel-glyph insertion works** for fonts whose kashida is a
  plain repeatable tatweel (Naskh/Kufi-style). Map U+0640 → gid once per run via
  Skrifa's `charmap`, read the advance via Skrifa's `glyph_metrics`, and splice
  `n` copies into the glyph stream at flagged clusters. No re-shaping.
- **(b) Re-shaping is only needed for "pretty" kashida** — fonts with curved
  connections, `jalt` justification-alternate glyphs, or a variation axis
  (Amiri, Gulzar's MSHQ). The `SAFE_TO_INSERT_TATWEEL` flag does not pick those;
  it guarantees base-text stability, not calligraphic quality.

### `jalt`, JSTF, `hb_shape_justify`
- Parley already forwards arbitrary OpenType features to HarfRust
  (`shape/mod.rs` builds `harfrust::Feature`s), so a `jalt` feature *could* be
  requested, but it is applied uniformly to the whole run, not per-elongation
  slot, so it isn't a justification control by itself.
- **HarfRust has no `hb_shape_justify` and does not consume the `JSTF` table.**
  (Grep of harfrust 0.10.0 source: the only "justification" is `apply_stch` in
  `ot_shaper_arabic.rs`, i.e. the font-driven `<stch>` stretch mechanism used
  for e.g. the Syriac Abbreviation Mark — not general kashida.) HarfBuzz's own
  `hb_shape_justify` (added 7.1.0) is experimental (`HB_EXPERIMENTAL_API`),
  works by iterating a *variable-font axis* to hit a target width, and is not
  ported to HarfRust. So **Parley must own the justification policy.**

### Arabic joining info
Not needed as a separate input: `SAFE_TO_INSERT_TATWEEL` already encodes the
valid, shaping-safe join positions (superior to computing joining types
ourselves, since it accounts for the font's actual behavior). A pure-Unicode
fallback (the `unicode-joining-type` crate or ICU joining-type data: insert
between a right-joining/dual-joining leader and a left-joining/dual-joining
follower) is possible but strictly worse and only worth it if we ever want
kashida without the produce-flag.

---

## 3. Skrifa's role

- **Tatweel glyph id:** `FontRef::charmap().map('\u{0640}')` → `GlyphId`
  (probe: 114 for Noto Kufi). Compute once per `RunData`.
- **Tatweel advance:** `FontRef::glyph_metrics(size, coords).advance_width(gid)`
  (probe: 171 upem → scaled by `font_size / units_per_em`, the same
  `scale_factor` already computed in `data.rs::push_run`).
- **Outline/rendering:** synthesized tatweel glyphs render through Parley's
  normal glyph path — they're ordinary `Glyph { id, x, y, advance }` values, so
  Skrifa/whatever back-end already draws them. Nothing new needed for outlines.
- **Variable-font elongation axes:** HarfRust applies variation coords **per
  run**, not per-glyph within a run (`ShaperInstance::from_variations`,
  `harf_shaper.coords()` are run-global; `RunData::coords_range` is one coord set
  per run). So an MSHQ-style "stretch this one join by axis value X" cannot be
  expressed inside a single run without re-shaping/splitting the run. **Implication:**
  variable-axis kashida (Gulzar-style) is out of scope for a non-re-shaping v1;
  it needs either per-slot run splitting + re-shape, or upstream
  `hb_shape_justify`-like support. (Gulzar is OFL, not Apache-2.0, and was not
  downloaded — see licensing note below.)

---

### 3.1 Why per-join variable-axis kashida needs re-shaping (detail)

The blocker is **not** that HarfRust inserts a shaping boundary when coords
differ — coords are not a per-glyph input at all, so there is no such event to
avoid. In HarfRust 0.10 the normalized coords live in a single
`ShaperInstance { coords: SmallVec<..> }` (`hb/face.rs:87`) that is bound to the
whole shaper (`.instance(Some(&instance))`) and therefore to the *entire*
`shape()` call. Neither `UnicodeBuffer` nor `ShapeOptions` carries any per-glyph
or per-cluster coordinate. So "give this one join a different MSHQ value" is an
**API impossibility** within a single shape call — the only way to get two coord
sets is to split the text into two `shape()` calls, i.e. a boundary *you*
introduce.

And that self-imposed split is **semantically incoherent** for a cursive join,
because coordinates drive far more than outlines:
- **Glyph selection** — `ShaperInstance::from_variations` precomputes a GSUB/GPOS
  `feature_variation_index` from the coords (`hb/face.rs:193-197`). That is the
  OpenType FeatureVariations / `rvrn` mechanism: a different axis value can
  activate a different conditionset and thus select different glyphs. An
  elongation-axis font may legitimately swap glyphs at axis ranges.
- **Positioning deltas** — GPOS applies coord-dependent deltas to
  `x_offset/y_offset/x_advance/y_advance` via `compute_delta(.., coords)`
  (`hb/ot/gpos/mod.rs:58-97`), and advances get HVAR deltas
  (`hb/glyph_metrics.rs:131-141`). Cursive attachment (the `attach_chain`/
  `attach_type` on `GlyphPosition`) and cross-join kerning are resolved **within
  one buffer only**.

So if you split at the join to vary coords: (i) the two seam glyphs are realized
from two different instances, so the kashida stroke's outline and advance don't
line up; (ii) cursive attachment and cross-boundary GPOS/kerning can't span the
seam. Buffer **pre/post-context** (`set_pre_context`/`set_post_context`, consumed
by the Arabic joining state machine at `hb/ot_shaper_arabic.rs:310-357` — the
same code that sets `safe_to_insert_tatweel`) *does* let each side still pick the
correct init/medi/fina **form**, but context is shaped under that run's own
coords and creates no cross-buffer positional attachment, so it doesn't rescue
(i) or (ii). Fundamentally you cannot *localize* the stretch to one join: coords
are global to the run, so a split makes the whole sub-run stretch, and the seam
breaks.

**What IS achievable** is whole-run (or whole-line) axis iteration — the
`hb_shape_justify` approach: pick one axis value for the entire run, re-shape,
measure, and binary-search the value to hit the target width. This is coherent
(coords stay global; the font's design spreads the elongation across its joins)
but (a) re-shapes per iteration per line, and (b) can't choose *which* join
elongates — the font decides. HarfRust exposes no `hb_shape_justify`, but it is
buildable on top of `ShaperInstance::from_coords` + re-shape + measure. This is
the natural high-quality follow-up path; v1 stays with tatweel insertion. (No
variable Arabic font was available under the Apache-2.0/Google-Fonts constraint,
and the vendored Noto Kufi Arabic is a static OTF with no MSHQ axis, so this
path was reasoned from source rather than measured.)

## 4. Experiments

Located in `kashida/experiments/` (kept as reference; standalone crates,
detached from the Parley workspace). See `experiments/README.md` and each
crate's `RESULTS.md`.

- **`harfrust_probe`** (built & run; output captured in `RESULTS.md`):
  - Confirms `SAFE_TO_INSERT_TATWEEL` is emitted only with the produce buffer
    flag, and lands on interior cursive-join clusters (not word-initial/final).
  - Confirms Skrifa resolves U+0640 → gid + advance.
  - **Confirms naive tatweel-glyph insertion == re-shaping** for Noto Kufi
    Arabic (identical base glyphs; +171 upem per tatweel), the central technical
    question for feasibility.

**Fonts / licensing.** The task constraint (download only from Google Fonts,
only if Apache-2.0) **could not be satisfied: there is no Apache-2.0 Arabic-script
font on Google Fonts** (the `apache/` dir has 44 families, none Arabic; all Noto
Arabic are OFL; Droid Arabic Naskh/Kufi were removed from the repo). Nothing was
downloaded. Experiments read, *by path only*, the OFL `NotoKufiArabic-Regular.otf`
already vendored at `parley_dev/assets/fonts/noto_fonts/` (not copied, not
redistributed). Details in `experiments/README.md`.

Not built (lower value given the probe already answers the key question, and no
extra experiment can change the licensing situation): a Parley-level render/
measure demo, and a Unicode-joining-type fallback detector. Both are described
above; the joining-type fallback is only relevant if we ever want kashida without
the produce-flag.

---

## 5. Concrete implementation plan for Parley

### 5.0 API surface
Add an opt-in justification mode rather than overloading `Alignment::Justify`
silently, so callers control cost and behavior:

```rust
// parley/src/layout/alignment.rs (or a new setting)
pub enum JustificationMode {
    /// Current behavior: distribute free space across inter-word spaces only.
    InterWord,
    /// Add kashida elongation at safe cursive joins, then inter-word spaces
    /// for any remainder.
    Kashida,
    // (future) KashidaThenSpace ratio / priority tuning.
}
```
Plumb it through the same builder/layout entry points that already carry
`Alignment` + `AlignmentOptions` (see `layout::alignment::align` callers and
`Layout::align`). Default = `InterWord` (no behavior change).

### 5.1 Shaping: request + capture the tatweel flag
File: `parley/src/shape/mod.rs`, `shape_item()`.
- After `buffer.set_direction(...)` set
  `buffer.set_flags(BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL)` (gate on
  "script is an elongation script" and/or "kashida mode is possible" to avoid
  the small cost otherwise — HarfBuzz notes the flag has a cost).

File: `parley/src/layout/data.rs`, `process_clusters()` + `push_cluster()`.
- `GlyphInfo` is already in hand in the per-glyph loop. OR
  `glyph_info.flags().is_safe_to_insert_tatweel()` into the cluster's flags.
- Add to `ClusterData`: `const KASHIDA_OPPORTUNITY: u16 = 4;` and set it in
  `push_cluster` when the (first/appropriate) glyph of the cluster carries the
  flag. (Bit 2 is free; `flags` is a `u16`.) The flag marks "a tatweel may be
  inserted before this cluster (logical order)".
- Store the run's **tatweel glyph id** and **scaled tatweel advance** once, in
  `RunData` (new fields, resolved via Skrifa in `push_run`, `None`/0 if the font
  has no U+0640). Needed so justification and rendering are O(1) per slot.

### 5.2 Line breaking: count opportunities per line
File: `parley/src/layout/line_break.rs`.
- Alongside `num_spaces`, accumulate `num_kashida_opportunities` on the line
  (count clusters with `KASHIDA_OPPORTUNITY` in the committed range, excluding
  trailing-whitespace region, mirroring the existing `num_spaces` handling near
  lines 563–632 / 1259–1284). Store on `LineData`.
- Policy choice (document it): typically at most one kashida per word, and/or
  prefer specific join priorities (e.g. before the final letter). A simple v1
  can treat every flagged cluster as eligible and distribute evenly; a v1.1 can
  rank.

### 5.3 Alignment: distribute elongation
File: `parley/src/layout/alignment.rs`, `align_impl`.
- In the `Alignment::Justify` arm, when `JustificationMode::Kashida`:
  1. Compute `free_space` as today.
  2. Decide the split between kashida and space (v1: kashida first up to a cap,
     spaces for the remainder; or a ratio). Let `kashida_space` be the portion
     assigned to elongation.
  3. Quantize kashida per slot to a whole number of tatweels:
     `n = round(kashida_per_slot / tatweel_advance)`, add
     `n * tatweel_advance` to that cluster's `advance`, and record `n` (e.g. a
     small count packed into `ClusterData` or a parallel per-cluster field) so
     rendering knows how many tatweel glyphs to synthesize. Feed any rounding
     remainder back into the space distribution.
  4. Keep the existing space loop for the remainder.
- **Undo:** the existing `UNDO_JUSTIFICATION` path must also zero the recorded
  tatweel counts and subtract the elongation. Because kashida quantizes, the
  undo can't just flip a sign on a float; store the applied per-cluster count and
  undo from that (or fully recompute). Update `unjustify()`/`is_aligned_justified`
  accordingly.
- RTL/LTR: reuse the existing forward/reverse item+cluster iteration; kashida
  slots are already in logical order per cluster, so no new direction logic.

### 5.4 Rendering: surface synthesized tatweel glyphs
Files: `parley/src/layout/cluster.rs::Cluster::glyphs()`,
`parley/src/layout/line.rs::GlyphRun::{glyphs, positioned_glyphs}` and
`GlyphRunIter`.
- `Cluster::glyphs()` currently yields either one inline glyph or a stored
  slice. Extend its `GlyphIter` to **append `n` tatweel glyphs** (id from
  `RunData`, `advance = tatweel_advance`, `x = y = 0`) for a cluster with a
  recorded kashida count. Because `positioned_glyphs()` advances the pen by each
  glyph's `advance`, the appended tatweels naturally occupy the added width and
  push following glyphs — no offset bookkeeping needed.
- Insertion side matters for correctness of the join: the flag is "insert
  *before* this cluster (logical)". In visual order that's on the leading
  (join) edge; verify against the probe's cluster/visual mapping when wiring it
  (RESULTS.md shows the tatweel landing between the joined letters). Get this
  direction right for both LTR-embedded and RTL runs.
- `GlyphRun`'s `glyph_start`/`glyph_count`/`advance` are computed by counting
  glyphs from `visual_clusters().flat_map(|c| c.glyphs())`; since the synthesized
  tatweels come from `Cluster::glyphs()`, the counts stay consistent as long as
  the same iterator is used everywhere (it is). Double-check `advance`
  accumulation includes them.

### 5.5 Interaction with existing space justification
- v1: kashida and space justification cooperate on one line (kashida first,
  spaces absorb the remainder), so a line with no kashida opportunities behaves
  exactly as today. This also handles mixed Arabic/Latin lines: Latin runs have
  no flagged clusters, so only their spaces stretch.
- Content-width / min-max width (`data.rs::calculate_content_widths`) is unaffected
  (kashida is applied at align time, not measured as intrinsic width) — but note
  that, like today's space justification, kashida is undone before re-line-break.

### Files that change (summary)
- `shape/mod.rs` — set produce-tatweel buffer flag.
- `layout/data.rs` — capture flag into `ClusterData.flags`
  (`KASHIDA_OPPORTUNITY`), add tatweel gid/advance to `RunData`, per-cluster
  applied-count storage.
- `layout/line_break.rs` — `num_kashida_opportunities` on `LineData`.
- `layout/alignment.rs` — `JustificationMode`, kashida distribution + undo.
- `layout/cluster.rs` + `layout/line.rs` — synthesize tatweel glyphs in the
  glyph iterators.
- Public API/builder — expose `JustificationMode`.

---

## 5.6 Rough LOC estimate

Ballpark (to the nearest ~100 lines), for an opt-in tatweel-based v1. These are
*added* lines against the files listed above; existing justification code is
reused, not rewritten.

| Component | File(s) | ~LOC |
|-----------|---------|------|
| `JustificationMode` API + builder/setting plumbing | `alignment.rs`, `builder.rs`, `setting.rs`, `lib.rs` | ~100 |
| Set produce-tatweel buffer flag (script-gated) | `shape/mod.rs` | ~20 |
| Capture flag → `ClusterData` flag; tatweel gid/advance on `RunData` (Skrifa); per-cluster applied-count | `layout/data.rs` | ~120 |
| `num_kashida_opportunities` per line | `line_break.rs` | ~50 |
| Kashida distribution + quantization + exact undo | `alignment.rs` | ~180 |
| Synthesize tatweel glyphs in glyph iterators + verify `GlyphRun` counts | `cluster.rs`, `line.rs` | ~80 |
| **Implementation subtotal** | | **~550** |
| Tests (shaping flag capture, distribution, undo, RTL, mixed runs) | `parley/src/tests`, `parley_tests` | ~250 |
| **Total** | | **~800** |

**Upstream (HarfRust/Skrifa): ~0 lines for v1** — HarfRust already exposes the
tatweel flag and Skrifa already exposes cmap + glyph advances, so no upstream
changes are required. Upstream work only becomes necessary for the
*out-of-scope* high-quality path (variable-axis / calligraphic kashida), which
would need a `hb_shape_justify`-style API or per-slot re-shaping support in
HarfRust — not estimated here.

So: **roughly 550 lines of implementation (~800 including tests), all in
Parley, none upstream.**

## 6. Open questions & risks

1. **Font quality / "ugly tatweel".** Flat U+0640 looks acceptable in
   Naskh/Kufi but poor in many display faces, and wrong where the font expects
   curved kashida or `jalt` alternates. `SAFE_TO_INSERT_TATWEEL` guarantees
   shaping stability, not aesthetics. HarfBuzz explicitly declined to detect
   "kashida-unsafe" fonts (issue #1503). Mitigation: keep kashida opt-in;
   consider a per-font allow/deny later.
2. **How much / where to elongate (policy).** HarfRust gives positions, not
   amounts or priorities. Good kashida distributes elongation by calligraphic
   rules (usually one, at a preferred join per word), not evenly across every
   safe slot. v1 even-distribution will look mechanical; ranking is a follow-up.
3. **Quantization.** Elongation is granular (whole tatweels of ~171 upem here),
   so lines won't hit the target width exactly; the remainder must fall back to
   space justification (or a fractional final tatweel via `x_advance` scaling,
   which risks a visible seam). Undo must be exact despite quantization.
4. **Variable-font / calligraphic kashida.** Out of scope for v1 (per-glyph
   variation within a run isn't expressible without re-shaping/run-splitting).
   Would need upstream `hb_shape_justify`-style support or a Parley re-shape
   path. Track HarfRust for a JSTF/justify API.
5. **Ligatures & marks.** Flagged clusters can be ligature starts or carry
   marks; ensure elongation is added to the join, and that inserting tatweels
   doesn't desync `ClusterData::glyph_len`/`glyph_offset` bookkeeping (the
   synthesized glyphs live only in the iterator, not the stored `glyphs` vec, to
   avoid disturbing offsets — chosen deliberately in 5.4).
6. **Cost.** The produce-tatweel flag adds shaping cost; gate it to elongation
   scripts and/or only when kashida mode is selected.
7. **Bidi / mixed runs.** Verify insertion side and iteration order for RTL runs
   embedded in LTR paragraphs and vice-versa (reuse existing bidi-aware loops).

## 7. References
- HarfRust 0.10.0 source (local): `hb/buffer.rs` (`GlyphFlags`,
  `SAFE_TO_INSERT_TATWEEL`, `PRODUCE_SAFE_TO_INSERT_TATWEEL`),
  `hb/ot_shaper_arabic.rs::apply_stch` (only built-in stretch; no JSTF/justify).
- HarfBuzz: `SAFE_TO_INSERT_TATWEEL` since 5.1.0; `hb_shape_justify` since 7.1.0
  (experimental, variable-axis based); `JSTF` table parsed but not used for
  justification (issues #1469, #1503).
- W3C "Approaches to full justification"; W3C Arabic gap analysis (ALReq);
  CSS Text 3 `text-justify: inter-character`.
- Experiments: `kashida/experiments/` (`harfrust_probe`).
