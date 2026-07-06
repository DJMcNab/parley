# Kashida Justification: General Research

Background research for implementing Arabic (kashida / elongation) justification in
Parley. This document surveys *how the problem is understood and solved elsewhere*: what
kashida is, how fonts express elongation, what shaping requires, how real engines behave,
and what a realistic implementation menu looks like ranked by fidelity vs. effort.

Throughout, **[documented]** marks claims backed by a spec or primary source, and
**[folklore]** marks widely-repeated claims that are plausible but under-documented. Sources
are listed inline and collected at the end.

---

## 1. What Kashida is

### 1.1 The core idea

Latin justification stretches the **whitespace between words** (and sometimes between
letters). Arabic script is cursive — most letters join to their neighbours along the
baseline — so the natural place to add width is *inside* words, by lengthening the
connecting strokes (the "cursive join") between letters. This elongation is the **kashida**
(Persian کشیده *kešide*, "stretched/drawn-out"); the equivalent Arabic term is **taṭwīl**
(تطويل, "lengthening"). [documented — TypoArabic pt.1; Wikipedia]

Crucially, elongation **does not change the meaning, pronunciation, or letter count** of a
word — it is purely a graphical device. [documented — MS Globalization]

### 1.2 Two very different mechanisms that get conflated

There are two distinct things people mean by "kashida", and confusing them is the source of
most implementation grief:

1. **Tatweel character insertion (U+0640 ARABIC TATWEEL).** A dual-joining "filler"
   character with a *fixed advance width* that you literally insert into the text/glyph
   stream between two joining letters. It draws a straight horizontal baseline segment.
   Simple, but crude: it is a 16th-century printing-press hack (compositors could not
   economically cut elongated metal sorts, so they padded with a straight bar), carried
   forward by typewriters and early digital systems. [documented — TypoArabic pt.1]

2. **True cursive elongation ("kashīda" proper).** The scribal practice: *specific parts of
   specific letters* swell along a curve, in context-sensitive shapes and lengths — the way
   a reed pen would draw them. Only some letters, and only certain strokes of them, may be
   elongated, and the shape depends on style (Naskh, Nastaʿlīq, Kufic…) and position.
   [documented — TypoArabic pt.1]

Tatweel is the lowest-fidelity approximation of kashīda. High-quality Arabic typography
wants the second thing; almost all shipping software does (a poor version of) the first.

### 1.3 Where elongation is permitted / preferred

- Elongation happens **at a cursive join between two letters that connect** — i.e. between a
  letter that joins on its left (in logical terms, connects to the following letter) and a
  following letter that joins on its right. It is a property of the *join*, not of a single
  glyph. [documented — Unicode Ch.9 joining model]
- It can **never** be inserted:
  - between two letters that do not join (e.g. after a right-joining-only letter),
  - inside a mandatory ligature (the classic case: **lam-alef** لا must not be split),
  - adjacent to non-joining characters or at word boundaries in a way that breaks shaping.
    [documented — Unicode Ch.9; TypoArabic; practical write-ups]
- Typographers **prefer a small number of well-chosen elongations per line** over many. The
  W3C ALReQ warns that "excessive use of kashida or applying very long kashidas results in
  uneven color", and that horizontal/vertical proximity of several kashidas looks unnatural.
  The strong convention is **at most one elongation per word**, placed by priority (§6).
  [documented — W3C ALReQ; folklore for the exact "one per word" rule]

### 1.4 Scripts affected

The cursive-joining model (and thus elongation) applies to the whole Arabic-script family
and other cursive scripts: **Arabic, Persian, Urdu, Syriac, N'Ko, Mandaic, Manichaean**, and
others. Unicode's `ArabicShaping.txt` defines `Joining_Type`/`Joining_Group` for all of
these. In practice, justification work centres on Arabic/Persian/Urdu. [documented —
Unicode `ArabicShaping.txt` header]

---

## 2. Font-level mechanisms for elongation

Ordered roughly from lowest to highest fidelity:

### 2.1 Tatweel glyph repetition (U+0640)

Insert one or more U+0640 into the text (or into the glyph run). Each has a fixed advance,
so line-length granularity is one tatweel-width. Every Arabic font has a tatweel glyph.
- **Pros:** universally available; trivial.
- **Cons:** straight bar only (ugly in Naskh/Nastaʿlīq); fixed granularity; pollutes the
  text (search, copy-paste, screen readers, reflow all degrade if inserted into the
  backing string); and — critically — **needs re-shaping** afterwards (§3). [documented —
  TypoArabic pt.1 & pt.2; lr0.org]

### 2.2 OpenType `jalt` — Justification Alternates

A GSUB type-1 (single-substitution) feature that maps a glyph (isolated/initial/medial/final
form) to a **wider alternate** form designed for justification. The layout engine turns it
on selectively, for chosen glyphs, to fill space — *not* globally. [documented — MS OT spec,
`features_fj`]
- **Pros:** designer-controlled, prettier than a straight bar; no text pollution.
- **Cons:** discrete widths (each alternate is a fixed size); requires the engine to apply
  the feature *per-glyph* and re-measure; browsers that expose it via
  `font-feature-settings:"jalt"` apply it to *all* glyphs at once, which is wrong (it should
  be selective). Few fonts ship meaningful `jalt` sets. [documented — TypoArabic pt.2]

### 2.3 OpenType `JSTF` — the Justification table

Purpose-built table (in the spec since ~1997) letting a font declare, per script/langsys,
ordered **justification priorities** and the GSUB/GPOS lookups (enable/disable
substitutions, tatweel insertion, alternate shaping) to apply at each priority level for
both **expansion** and **shrinkage**. It is the "correct" OpenType answer. [documented — MS
OT spec, `jstf`]
- **Reality: near-zero adoption.** A 30-year standoff — engines don't read `JSTF`,
  foundries don't populate it, nobody prioritizes breaking the deadlock. HarfBuzz does not
  implement it. Treat `JSTF` as *documented but effectively dead*. [documented —
  TypeDrawers "Making JSTF better"; TypoArabic pt.2; lr0.org]

### 2.4 Variable fonts with an elongation axis (the modern high-fidelity path)

A variable-font design axis continuously morphs letterforms to elongate cursive strokes.
Because it is continuous, the engine can dial in *exactly* the width a line needs, with
genuinely curved, calligraphic kashidas — and **spaces stay ideal**. This is currently the
cleanest technical solution. [documented — lr0.org; jmsole/gext-demos]

Axes/fonts that actually exist (none of these axis tags are *registered* in the OT spec —
they are custom/foundry tags):
- **`MSHQ` ("Mashq")** — elongation axis in **Raqq** (Khaled Hosny's early-Kufic manuscript
  typeface). In Kufic the elongatable letters are the horizontal ones — *dal, tah, kaf, sad*,
  plus *beh* and *feh* in initial/medial positions. [documented — aliftype.com/raqq]
- **`GEXT` ("glyph extension")** — a de-facto convention for a justification/elongation axis;
  demoed by Jindřich Šesták's `gext-demos` doing full justification with long curved
  kashidas. [documented — github.com/jmsole/gext-demos]
- **Amiri (2022 rewrite)** — carries a *curvilinear* kashida: fed elongation, it substitutes
  graded, swelling curved strokes in ~4 sizes, "the way the pen would". Uses OT features
  rather than a simple axis. [documented — TypoArabic; lr0.org]
- **Gulzar** (Nastaʿlīq) and others explore contextual elongation; **Readex Pro** is a
  Latin+Arabic variable family but its axes are weight/optical, *not* an elongation axis —
  do not assume it justifies. (Common misconception — verify axes before relying on any
  specific family.) [folklore/caveat]

Caveat: an implementer cannot rely on any particular axis tag. You must *probe the font's
`fvar`* for a known-elongation axis (`MSHQ`, `GEXT`, sometimes `wdth` misused) and have
per-font knowledge, because there is no standard tag. [documented — v-fonts kashida tag list;
gext-demos README]

### 2.5 Summary table of font mechanisms

| Mechanism | Fidelity | Granularity | Font support | Text pollution | Engine work |
|---|---|---|---|---|---|
| Tatweel U+0640 insertion | Low (straight bar) | Fixed step | Universal | Yes (if in backing text) | Re-shape needed |
| `jalt` alternates | Medium | Discrete sizes | Rare | No | Per-glyph GSUB + remeasure |
| `JSTF` table | High (by design) | Font-defined | ~None | No | Full JSTF engine (nobody has) |
| Variable elongation axis | Highest | Continuous | Growing but niche | No | Axis probe + interpolate + remeasure |

---

## 3. Shaping considerations

This is where naive implementations break.

### 3.1 Joining-type analysis (Unicode)

`ArabicShaping.txt` assigns each character a **`Joining_Type`**:
- **`D` Dual_Joining** — joins on both sides (e.g. beh, teh, seen, most letters). U+0640
  TATWEEL is itself `D`.
- **`R` Right_Joining** — joins only on the right (toward preceding letter): the six
  "non-connecting-forward" letters **alef, dal, dhal, reh, zain, waw** (and variants).
- **`L` Left_Joining** — rare.
- **`C` Join_Causing** — like tatweel/ZWJ, forces a join without a form of its own.
- **`U` Non_Joining**, **`T` Transparent** (marks/diacritics — invisible to joining).
  [documented — Unicode Ch.9 Tables 9-3/9-7; `ArabicShaping.txt`]

**Elongation is valid only at a join between a letter that connects on its left and a
following letter that connects on its right** — i.e. essentially at joins involving dual-
joining letters. After a right-joining-only letter (alef, dal, reh, waw, …) there is *no*
join to the next letter, so no elongation there. `Transparent` (mark) characters must be
skipped when computing adjacency. [documented — Unicode joining model]

### 3.2 The re-shaping problem (the #1 gotcha)

If you insert a tatweel into the text and **do not re-run the shaper**, contextual shaping
breaks: many fonts use contextual GSUB/`calt`/cursive-attachment (`curs`) so that adjacent
glyphs are chosen and positioned relative to each other. A tatweel jammed in without
re-shaping can leave glyphs mis-selected or off the baseline. The correct sequence is:
decide join + count → insert tatweel(s) → **re-shape the run** → re-measure. [documented —
lr0.org; ar-ms.me; HarfBuzz issue #1503]

Two escape hatches from full re-shaping:
- **Advance-width adjustment instead of insertion.** Some engines widen the *advance* of the
  glyph before the join and draw the kashida as the "extra" width (this is how Uniscribe's
  model conceptually works — extra advance width is rendered as a kashida for Arabic runs).
  This avoids editing the text buffer but only produces a straight bar and still needs care
  with cursive-attachment fonts. [documented — MS `ScriptJustify` / miloush.net]
- **Variable-axis / `jalt`**: re-shape/re-interpolate the affected glyphs only.

### 3.3 HarfBuzz's actual position

- HarfBuzz **shapes** Arabic correctly (positional forms `isol/init/medi/fina`, ligatures
  `rlig`/`liga`, marks `mark`/`mkmk`, cursive `curs`) but **does not implement
  justification** — "it's a hard problem, and is under-specified in OpenType." No
  justification API, no `JSTF`. [documented — lr0.org; HarfBuzz #1503]
- There is a long-standing feature request (#1503) for a public API just to *ask whether a
  font can add a kashida / where* — still unresolved. HarfBuzz added a
  `hb_shape_justify()`-style experimental API for *variable-font* justification (varying an
  axis to hit a target width) but tatweel-insertion policy is left to the caller. [documented
  — HarfBuzz issue tracker]
- Consequence for Parley (which shapes via HarfBuzz/`swash`/`rustybuzz`): **the shaper will
  not choose kashida points for you.** You must decide join points from Unicode joining data
  + per-cluster info, then either insert-and-reshape or drive a variable axis.

### 3.4 Ligatures, marks, diacritics

- **Never split a required ligature** (lam-alef). Track ligature clusters and forbid an
  elongation point inside them. [documented — Unicode; practical]
- **Marks/diacritics** (`Transparent`) sit on a base letter; when the base's join is
  elongated, marks must stay anchored to the base. Elongating a join *between* bases (not
  under a mark) is safest. Uniscribe exposes `fDiacritic` per glyph precisely so callers can
  avoid elongating awkwardly around diacritics. [documented — miloush.net / `SCRIPT_VISATTR`]

---

## 4. How real engines do it (or don't)

| Engine / stack | Kashida support | Notes |
|---|---|---|
| **Windows Uniscribe (`usp10`)** | Yes — the reference implementation | `ScriptShape` fills a per-glyph `SCRIPT_VISATTR.uJustification` field with a `SCRIPT_JUSTIFY_*` class (§5); `ScriptJustify` distributes width giving **top priority to kashida**, then inter-word blanks, then inter-character. Callers may edit `uJustification` to tune. [documented — MS docs; miloush.net] |
| **Windows DirectWrite** | Partial/limited | Modern successor; Arabic justification support is weaker/less exposed than classic Uniscribe. [folklore/partial] |
| **Microsoft Word** | Yes — Justify **Low/Medium/High** | Tatweel-based; "High" uses wider kashidas than "Low". Historically the best mainstream implementation. [documented — MS Globalization] |
| **Internet Explorer ≤ Trident** | Yes — `text-justify: kashida` | IE 5.5 shipped real kashida justification; for a while the *only* on-screen correct engine. Non-standard `text-kashida-space` too. Gone in modern Edge (Chromium). [documented — lr0.org; MS legacy] |
| **Adobe InDesign ME / World-Ready composer** | Yes, most options of any DTP tool | Kashida length Short/Medium/Long + "Justification Alternates (Naskh)". Best-in-class results come from **DecoType Tasmeem** (a plug-in using DecoType's ACE engine) — genuine multi-technique justification. Adobe's built-in options are famously undocumented and imperfect. [documented — TypoArabic pt.2; khtt.net] |
| **DecoType Tasmeem / Mushaf Muscat** | Yes — the gold standard | ~35 years of R&D; dynamic engine combining real kashida elongation, glyph alternates, swashes, and spacing. Not a general library. [documented — TypoArabic pt.2] |
| **LibreOffice** | Partial | Uses HarfBuzz; offers tatweel-like kashida insertion comparable to word processors, but AAT/Graphite justification is whitespace-only in aligned text. [documented — TypoArabic pt.2] |
| **Chrome/Blink, Firefox/Gecko, Safari/WebKit** | **No real kashida** | `text-align: justify` on Arabic falls back to **inter-word spacing only** → ragged look. `text-justify: inter-character`/`distribute` exist but don't do kashida. `text-justify: kashida` is **not** implemented (only legacy IE had it). WebKit bug #6203 and Mozilla bug #185600 (request kashida justify) are ancient and unfixed. CSSWG has an open Arabic-justification issue since ~2015. [documented — bug trackers; lr0.org; tutorialpedia] |
| **Android / Minikin** | No kashida | Minikin (line breaker + justifier) does whitespace/inter-word justification; no documented kashida elongation. [documented — LWN "Rethinking text layout in Android"; absence in docs] |
| **HarfBuzz** | Shaping yes, justification no | See §3.3. [documented] |
| **TeX / LaTeX** | Via packages | e.g. stretchable-kashida approaches; not built-in. [documented — andreasmhallberg.github.io] |

**CSS `text-justify` status:** registered values are `auto | none | inter-word |
inter-character` (and `distribute`, being folded into `inter-character`). There is **no
standardized `kashida` value** in the current spec; the only engine that ever shipped one was
IE. So for the web there is no portable kashida mechanism today. [documented — CSS Text
module; browser-support write-ups]

---

## 5. Layout considerations

### 5.1 Justification runs *after* line-breaking — but Arabic couples them

General model: choose line breaks first, then justify each line to the measure. But Arabic
has a twist that Latin doesn't:

> **A line's stretch capacity depends on *which letters landed on it*, not on how many words
> it has.** A line full of dual-joining pairs can stretch a lot; a line dominated by the six
> non-joining letters can barely stretch at all.

So an ideal Arabic justifier couples break-point selection with elongation capacity (a
Knuth-Plass-style model where "glue" stretch is per-join and content-dependent). Most
shipping engines *don't* do this — they break like Latin, then elongate whatever the line
allows. A pragmatic implementation can do the same and accept the quality ceiling. [documented
— lr0.org]

### 5.2 Mixing kashida with inter-word (space) justification

The residual width to fill on a line is distributed across mechanisms in a priority order.
Uniscribe's order is the canonical reference: **kashida first, then Arabic blanks (word
spaces), then inter-character.** [documented — MS `ScriptJustify`]

But typographic advice (W3C ALReQ) is that *no single mechanism should carry all the load* —
combine kashida + inter-word + intra-word + alternates + ligatures so each stays within
tasteful limits, avoiding both over-long kashidas and gaping word spaces. The per-script/
per-document balance is a policy decision the engine should ideally expose. [documented — W3C
ALReQ]

Practical heuristics seen in the wild [folklore, but consistent across sources]:
- Distribute *some* to kashida and *some* to spaces rather than exhausting one first.
- Cap kashida length (a max elongation per join / per line) to protect "colour".
- At most one kashida per word.

### 5.3 Bidi / mixed-script lines

Only **elongate the Arabic-script runs**. In a bidi line with Latin/digits, the Latin runs
justify with normal inter-word spacing; kashida applies solely within Arabic runs. This
requires run segmentation by script (which Parley already does for shaping) and applying the
right justification strategy per run. RTL specifics: elongation is inserted at the cursive
join in *logical* order, but rendered RTL — work in logical order and let the shaper/bidi
handle visual order. [documented — general bidi + Unicode]

### 5.4 Per-cluster data the engine needs

To place elongations you need, per cluster / glyph-run, at minimum:
- **"Can elongate after this cluster?"** — derived from joining types of this cluster's last
  base char and the next cluster's first base char (a real cursive join, not a ligature
  interior, not adjacent to a mark that would be orphaned).
- **A priority class** for that join (§6), so higher-quality points win.
- Whether the cluster is part of a **required ligature** (forbid interior points).
- The **font's elongation capability** at that point (does the font have a tatweel? a `jalt`
  alternate? a variable axis? what min/step/max width?).

Uniscribe packages exactly this into `SCRIPT_VISATTR` (a `uJustification` class + a
`fDiacritic` flag) per glyph. A Parley design would attach analogous per-cluster flags during
shaping/analysis. [documented — MS `SCRIPT_VISATTR`]

---

## 6. Quality / priority algorithms

### 6.1 Uniscribe's `SCRIPT_JUSTIFY_*` classes (the documented reference)

`ScriptShape` tags each glyph with one of these justification classes (higher = more
preferred elongation point). This is the closest thing to an *official* priority list.
[documented — MS `SCRIPT_JUSTIFY` enum, `usp10.h`]

| Value | Constant | Meaning |
|---|---|---|
| 0 | `SCRIPT_JUSTIFY_NONE` | No justification at this glyph |
| 1 | `SCRIPT_JUSTIFY_ARABIC_BLANK` | A blank (space) inside an Arabic run |
| 2 | `SCRIPT_JUSTIFY_CHARACTER` | Inter-character justification point follows |
| 3 | `SCRIPT_JUSTIFY_RESERVED1` | reserved |
| 4 | `SCRIPT_JUSTIFY_BLANK` | Blank outside an Arabic run (normal word space) |
| 5–6 | `RESERVED2/3` | reserved |
| 7 | `SCRIPT_JUSTIFY_ARABIC_NORMAL` | Normal middle-of-word glyph connecting to the right |
| 8 | `SCRIPT_JUSTIFY_ARABIC_KASHIDA` | Existing kashida (U+0640) mid-word |
| 9 | `SCRIPT_JUSTIFY_ARABIC_ALEF` | Final form of alef-like (U+0627/0625/0623/0622) |
| 10 | `SCRIPT_JUSTIFY_ARABIC_HA` | Final form of heh (U+0647) |
| 11 | `SCRIPT_JUSTIFY_ARABIC_RA` | Final form of reh (U+0631) |
| 12 | `SCRIPT_JUSTIFY_ARABIC_BA` | Final form of beh (U+0628) |
| 13 | `SCRIPT_JUSTIFY_ARABIC_BARA` | Beh-reh–like ligature (U+0628,U+0631) |
| 14 | `SCRIPT_JUSTIFY_ARABIC_SEEN` | **Highest priority:** initial form of seen class (U+0633) |
| 15 | `SCRIPT_JUSTIFY_ARABIC_SEEN_M` | **Highest priority:** medial form of seen class |

Note this enum encodes *what kind of point it is*, and Microsoft's justifier maps those to a
preference order (seen/sad joins rank highest). The numeric value is a class id, not a strict
"higher number wins" scale — but seen (14/15) is explicitly documented as highest priority.

### 6.2 Microsoft's kashida placement priority (the "Big Kashida Secret")

Microsoft *published* an ordered priority scheme (Adobe kept theirs secret). Algorithm:
1. In each word, find the **highest-priority** eligible join.
2. Attach the kashida to that join.
3. If several joins tie at the same priority within a word, place the kashida **toward the
   end of the word**.
[documented — khtt.net "The Big Kashida Secret"]

**Priority order (highest → lowest):** [documented — khtt.net]

| Rank | Where the kashida goes |
|---|---|
| 1 | A **manually inserted** kashida (user/authoring intent) — always honoured first |
| 2 | After **seen (س) or sad (ص)**, initial/medial forms (the "teeth" join) |
| 3 | Before the **final form of taa marbuta (ة), heh (ه), dal (د)** |
| 4 | Before the **final form of alef (ا), tah-lam (لا)‡, kaf (ك), gaf (گ)** |
| 5 | Before the **medial form of beh (ب), reh (ر), yeh (ي), alef maqsura (ى)** |
| 6 | Before the **final form of waw (و), ain (ع), qaf (ق), feh (ف)** |
| 7 | Before **other final-form connectable characters** |

‡ "tah-lam" here refers to the lam in lam-alef context; care needed not to split the actual
lam-alef ligature. (The rank-4/5 letter groupings are as reproduced by secondary sources;
treat exact letters as *approximate* — the underlying primary matrix is not fully public.)
[folklore-adjacent: reproduced consistently but from secondary sources]

### 6.3 Common heuristics / limits (mostly folklore, but consistent)

- **One kashida per word** (default); more only under "Justify High".
- **Maximum elongation cap** per join and per line to preserve even colour.
- Prefer distributing across *several words* on a line over one giant kashida.
- Never elongate: ligature interiors (lam-alef), after non-joining letters, at word ends
  facing a space (that's inter-word's job), around orphaned marks.
- Justify-Low/Medium/High = increasing max kashida width budget. [documented — MS; folklore
  for details]

---

## 7. Realistic implementation options for Parley, ranked

Parley shapes via HarfBuzz-family shapers and already segments runs by script and exposes
per-cluster data — so the analysis scaffolding (joining types, run boundaries) is within
reach. HarfBuzz will *not* pick kashida points for us (§3.3).

Ranked by fidelity vs. effort:

### Option A — Inter-word (space) justification only (baseline, lowest effort)
Do what Chrome/Firefox/Android do: justify Arabic by stretching word spaces. No kashida at
all. Correct, ships today, universally safe, but not "true" Arabic justification. Good
**Phase 0**. Effort: low. Fidelity: low (but matches the web baseline).

### Option B — Tatweel insertion + re-shape (classic, medium effort, medium fidelity)
1. After line-breaking, for each Arabic run compute eligible joins from Unicode joining data.
2. Rank with the priority table (§6); pick ≤1 point per word by priority; compute how many
   tatweel-widths (or which single insertion) hit the target width.
3. Insert U+0640 into the shaping input **for layout only** (keep the backing string clean to
   avoid search/copy/a11y pollution), **re-shape**, re-measure, iterate/adjust.
4. Blend with inter-word spacing (Uniscribe order: kashida → spaces → inter-char), capped.

Pitfalls: fixed granularity (one tatweel width); straight bars only; must re-shape; must not
split lam-alef; must skip marks. This is the Uniscribe/Word-class result. Effort: medium.
Fidelity: medium.

### Option C — Advance-width kashida (no text edit) (medium effort, medium fidelity)
Instead of inserting tatweel glyphs, widen the advance of the glyph before a chosen join and
draw a kashida bar to fill (Uniscribe's conceptual model). Avoids text-buffer edits and
partial re-shaping, but only yields straight bars and needs care with cursive-attachment
fonts (`curs`). Effort: medium. Fidelity: medium.

### Option D — Variable-font elongation axis (higher effort, highest fidelity)
Probe each font's `fvar` for an elongation axis (`MSHQ`, `GEXT`, occasionally misused
`wdth`). At chosen joins, interpolate the axis to hit the exact target width (continuous —
no granularity problem), re-shape/re-measure. Genuinely curved, calligraphic kashidas; spaces
stay ideal. Limited to fonts that have such an axis (Raqq, some Amiri configs, GEXT demos) —
so it must **fall back** to Option A/B for other fonts. HarfBuzz's experimental
variable-justify API can help drive the axis. Effort: higher. Fidelity: highest available.

### Option E — `jalt` alternates (medium effort, niche)
Apply the `jalt` GSUB feature *selectively* to chosen glyphs to swap in wider forms. Discrete
sizes, few fonts ship it, and you must apply it per-glyph (not globally like browsers wrongly
do). Useful as an adjunct to D. Effort: medium. Fidelity: medium, font-dependent.

### Not recommended
- **`JSTF` table engine** — "correct" but effectively no fonts populate it and no engine
  reads it; huge effort for ~zero real-world payoff.

### Suggested phasing
1. **Phase 0:** Option A (inter-word) — unblocks Arabic justification immediately.
2. **Phase 1:** Analysis layer — per-cluster "can-elongate + priority" flags from Unicode
   joining data, run/ligature/mark awareness. This is the reusable core for B/C/D.
3. **Phase 2:** Option B or C (tatweel) with priority selection + re-shape + blend with
   spaces + caps.
4. **Phase 3:** Option D (variable axis) with per-font capability probing and graceful
   fallback; optionally E as an adjunct.

The hard, high-value engineering is the **analysis layer** (Phase 1) and the
**re-shape/re-measure loop** — both are prerequisites for any real-kashida option and are
where correctness (joining types, ligatures, marks, bidi runs) lives.

---

## 7a. Fidelity of whole-run axis binary-search vs. per-join control

A tempting shortcut for variable-axis justification (the `hb_shape_justify` style): pick
**one** elongation-axis value for the entire run/line, re-shape, measure, and binary-search
the axis value until the line hits the target width. What fidelity does this lose?

**Short answer:** the font *does* localize deformation correctly *within each glyph* — a
single axis value only moves the strokes that may sensibly stretch, and non-stretchable
letters keep their skeleton. What one global value *cannot* do is localize the **selection
across the line**: it elongates **every** eligible join at once, in lockstep, which is the
opposite of the calligraphic convention. The lost fidelity is in *selection/distribution*,
not in per-glyph deformation.

**What the font sources actually show:**
- **Raqq / `MSHQ` is genuinely a single global control.** Only a subset responds — *dal,
  tah, kaf, sad*, and *beh/feh* in initial/medial position (isolated *ain* / isolated-final
  *hah* expand slightly, final *alef* shrinks slightly). Everything else does **not** vary;
  Raqq's docs say the rest "do not elongate but can be elongated by inserting tatweel after
  them when needed." Raising `MSHQ` moves **all** eligible forms across the whole text at
  once — no per-word/per-join granularity. [documented — aliftype.com/raqq]
- **The GEXT justification demo deliberately avoids the global approach.** To get per-join
  control the authors "duplicated each word with a kashida and made any redundant characters
  invisible while accounting for their width," then vary `GEXT` **per glyph**; only kashida
  glyphs + word-spacing participate. Strong practitioner evidence that a single global value
  was considered inadequate and they engineered per-join application on top of the axis.
  [documented — jmsole/gext-demos]

**Concrete fidelity losses of "one axis value for the whole line + binary search":**
1. **Violates "≤1 kashida per word."** A global axis stretches every eligible join,
   including multiple joins inside one word → busy/"combed" texture instead of a few
   deliberate elongations. [W3C ALReQ; khtt.net; TypoArabic]
2. **No priority selection.** Can't say "stretch this seen a lot, leave that beh alone" —
   all joins move together, proportionally. [khtt.net priority scheme]
3. **Content-dependent over/under-stretch → uneven colour down the column.** Stretch capacity
   depends on which letters landed on the line (lr0.org). A line with few axis-responsive
   joins needs a large axis value → those few joins get grotesquely over-stretched; a
   letter-dense line reaches target at a tiny value. Same target fill, wildly different
   per-join stretch line-to-line — the "uneven colour / unnatural proximity" ALReQ warns of,
   with no way to cap an individual join.
4. **Short/sparse lines get rubber-banded.** Binary search only turns the knob, so it maxes
   the axis on a 1–2-join line instead of spilling excess into word spacing.
5. **Axis has a design ceiling and a limited letter set.** A line whose stretchable letters
   all fall outside the axis set can't be justified by the axis at all (Raqq falls back to
   tatweel there); and each stroke has a max, so a sparse line may not reach target width via
   axis alone → must still blend with spacing/tatweel.

**Nuance:** the intuition is partly right — the font *does* prevent nonsense (only correct
strokes move; letters that shouldn't stretch don't). A global axis is far better than scaling
advances or stretching outlines. But "localized to strokes that can stretch" ≠ "localized to
the joins a typographer would choose." The loss is uniform-all-eligible-joins vs.
selective-one-per-word-by-priority, plus no capping/redistribution.

**Takeaway for Parley:** the high-fidelity variable path is *not* "one value for the whole
run + binary search" — it's the gext-demos shape: choose join points by priority (≤1/word),
apply the axis (or tatweel) *per chosen join*, and blend with inter-word spacing so no single
join over-stretches. Whole-run binary search is the cheap tier — an acceptable step up from
inter-word-only, but distinctly lower fidelity than per-join control, not equivalent to it.

---

## 7b. Are elongation axes (GEXT/MSHQ) available on Google Fonts? — No

Investigated 2026-07-04. **No font hosted on Google Fonts exposes a `GEXT`, `MSHQ`, or any
Arabic elongation/kashida/justification axis.** Evidence:

- **GF Axis Registry** (`google/fonts` → `axisregistry/Lib/axisregistry/data/`, 78 axis
  definitions) contains no elongation/kashida axis. The only vaguely-related tags are
  unrelated to cursive elongation: `y_vertical_extension` (vertical height), `element_expansion`
  / `hyper_expansion` (expressive/paint display fonts). No `GEXT`, no `MSHQ`, no `extension`
  in the Arabic sense.
- **GitHub code search across all of `google/fonts` for `GEXT` and for `MSHQ` → 0 hits.**
  This covers both the registry `.textproto` tag fields and every family's `METADATA.pb`
  (which lists each family's axis tags), so no hosted family declares those axes.
- **Google Fonts policy makes this structural, not incidental.** GF only serves *registered*
  axes: axes present in a font but absent from the GF Axis Registry "will not function via
  the API," and submitters are told to **strip unregistered custom axes at the binary level**
  (fonttools instancer) before onboarding. So even if an upstream Arabic font shipped a
  `GEXT`/`MSHQ` axis, the Google-Fonts-hosted build would have it removed until/unless the
  axis were formally registered — and neither is. [google/fonts issue #2846; gf-guide axis-registry]

**Where the elongation-axis fonts actually live (not Google Fonts):** `MSHQ` is used by
**Raqq** (aliftype — `aliftype/raqq` on GitHub, **AGPL-3.0** — verified via the repo's LICENSE,
2026-07-04; an earlier draft of this section wrongly said OFL); `GEXT` appears in
**jmsole/gext-demos** proof-of-concept fonts (**no license file or statement in the repo** —
all-rights-reserved by default, reference-only); **Amiri**'s curvilinear kashida ships under
**OFL-1.1** from its own site/repo (verified). Code search confirms `MSHQ` occurrences on
GitHub are in `aliftype/raqq` / `aliftype.github.io` (the rest are unrelated noise) — none in
`google/fonts`.

**Implication for Parley:** a variable-axis kashida path (§7 Option D) cannot rely on
Google-Fonts-served fonts today; it requires bringing fonts in directly, and licensing is a
real constraint there: Amiri (OFL) is usable for testing/bundling; Raqq's AGPL-3.0 is
problematic for most redistribution/bundling scenarios; the gext-demos fonts are unlicensed.
For the general Google-Fonts Arabic catalogue, only tatweel-insertion (§7 B/C) or inter-word
(§7 A) are viable, since those families expose no elongation axis.

---

## 8. Key open questions / things to verify before building

- **Which fonts Parley's users actually use** — determines whether Option D is worth it or
  whether tatweel (B/C) is the realistic ceiling.
- **Parley's per-cluster data model** — does it already surface enough (base codepoints,
  ligature membership, mark/`Transparent` status) to compute join eligibility without
  re-deriving from source text?
- **Re-shaping cost** — how expensive is a re-shape per justified line in Parley's pipeline,
  and can it be scoped to only affected runs?
- **Coupling with line-breaking** — accept the "break like Latin, then elongate" quality
  ceiling (pragmatic) vs. content-dependent stretch in the breaker (Knuth-Plass-style, much
  harder).
- **The exact priority letter-groups (§6.2 ranks 3–6)** are from secondary sources; if
  fidelity matters, cross-check against a primary corpus / calligraphic reference before
  encoding them.

---

## Sources

- Microsoft, *Text justification* (Globalization): https://learn.microsoft.com/en-us/globalization/fonts-layout/text-justification
- Microsoft, `SCRIPT_JUSTIFY` enum (usp10.h): https://learn.microsoft.com/en-us/windows/win32/api/usp10/ne-usp10-script_justify
- Microsoft, `ScriptJustify` function: https://learn.microsoft.com/en-us/windows/win32/api/usp10/nf-usp10-scriptjustify
- Microsoft, *Using Uniscribe*: https://learn.microsoft.com/en-us/windows/win32/intl/using-uniscribe
- Microsoft, OpenType `JSTF` table: https://learn.microsoft.com/en-us/typography/opentype/spec/jstf
- Microsoft, OpenType registered features f–j (`jalt`): https://learn.microsoft.com/en-gb/typography/opentype/spec/features_fj
- Microsoft, *Developing OpenType Fonts for Arabic Script*: https://learn.microsoft.com/en-us/typography/script-development/arabic
- Michael Kaplan (miloush.net mirror), *And how exactly do you justify those frigging kashidas?*: http://archives.miloush.net/michkap/archive/2010/08/31/10056140.html
- Khatt Foundation, *The Big Kashida Secret*: https://www.khtt.net/en/page/1821/the-big-kashida-secret
- TypoArabic (Univ. of Reading), *On Arabic justification, part 1 – a brief history*: https://research.reading.ac.uk/typoarabic/on-arabic-justification-part-1/
- TypoArabic, *On Arabic justification, part 2 – software implementations*: https://research.reading.ac.uk/typoarabic/on-arabic-justification-part-2-software-implementations/
- "La Vita Nouva" (lr0.org), *An interactive introduction to … rendering Arabic typography and its technical debt*: https://lr0.org/blog/p/arabic/
- Abdul Rahman Sibahi, *Thoughts on (practical) Arabic Justification*: https://ar-ms.me/thoughts/practical-arabic-justification/
- W3C, *Arabic & Persian Layout Requirements (ALReQ)*: https://www.w3.org/International/alreq/ and https://www.w3.org/TR/alreq/
- Unicode Standard, Chapter 9 (Middle Eastern / Arabic cursive joining): https://unicode.org/versions/Unicode16.0.0/core-spec/chapter-9/
- Unicode `ArabicShaping.txt`: https://www.unicode.org/Public/15.1.0/ucd/ArabicShaping.txt
- HarfBuzz issue #1503 (API to check kashida support): https://github.com/harfbuzz/harfbuzz/issues/1503
- TypeDrawers, *Making JSTF better*: https://typedrawers.com/discussion/3465/making-jstf-better
- Raqq typeface (MSHQ axis): https://aliftype.com/raqq/english.html
- jmsole/gext-demos (GEXT axis justification demos): https://github.com/jmsole/gext-demos
- Variable fonts tagged "kashida": https://v-fonts.com/tags/C92
- WebKit bug #6203 (Kashida full justification): https://bugs.webkit.org/show_bug.cgi?id=6203
- Mozilla bug #185600 (Arabic justify should use Tatweel): https://bugzilla.mozilla.org/show_bug.cgi?id=185600
- Wikipedia, *Kashida*: https://en.wikipedia.org/wiki/Kashida
- Andreas Hallberg, *Stretchable kashida and Arabic text justification in LaTeX*: http://andreasmhallberg.github.io/stretchable-kashida/
- Typst issue #740 (Justification system of Arabic script): https://github.com/typst/typst/issues/740
- LWN, *Rethinking text layout in Android and beyond* (Minikin): https://lwn.net/Articles/662569/
