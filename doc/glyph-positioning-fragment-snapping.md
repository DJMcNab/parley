# Fragment-origin snapping (bring-up step B13)

Implementation record for the last open bring-up step of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md): the
1/64 px ceil-snapping Blink applies to every fragment origin on a line. This is
what had `MAX_RUNS` pinned to 1.

**Status: landed.** The model lives in
`parley_glyph_positioning_extract::parley_output`, the golden schema is
fragment-structured, `MAX_RUNS` is back to 4, and the corpus is 50 passing plus 5
`known_failing/`.

## The model

Blink lays a line out as a sequence of **fragments**, each positioned at the
ceil-snapped end of the previous one:

```
origin_0     = the line's content edge
origin_{k+1} = origin_k + ceil64(width_k)
```

`width_k` is the fragment's own unrounded advance sum. This is
`ShapeResult::SnappedWidth()` / `ShapeResultView::SnappedWidth()`, both
`LayoutUnit::FromFloatCeil(width_)` (`shape_result.h:170`,
`shape_result_view.h:124`). Because `origin_k` is itself already on the 1/64 grid,
"ceil each width then sum" and "ceil the running total" are the same function —
there is no ambiguity to resolve there.

Every prediction was reproduced to the last bit against live Chrome (Chrome for
Testing 151.0.7922.77):

| case | Chrome | `origin_k + ceil64(width_k)` |
| --- | --- | --- |
| 2 spans, 1 line | `57.9062` | `ceil64(57.898438) = 57.90625` |
| 4 spans, 1 line | `94.0312`, `172.109` | `94.03125`, `172.109375` |
| 4 spans, wrapping | `78.5156`, `140.016`, `64.0938` | `78.515625`, `140.015625`, `64.09375` |

The recurrence is the easy half. What a fragment *is* took three rules, all found
empirically and all documented on `fragments_of` in the extract crate:

1. **A fragment is one span, not one shaping run.** Blink's `InlineItemsBuilder`
   splits at DOM boundaries; the `RunSegmenter`'s script and font segmentation
   happens *inside* one item's `ShapeResult`. Parley's `Line::items` is the finer
   split. The sampling alphabet is deliberately multi-script, so a single-span
   golden routinely arrives as 18 glyph runs; snapping at each is wrong by ~0.1 px,
   roughly six times the error being modelled. Getting this wrong broke all 15
   passing goldens before it was fixed, which is how it was found.
2. **A soft-wrapped line's hanging trailing whitespace is a fragment of its own** —
   but only when the break splits a span, and never on the last glyph-bearing line.
3. **Two adjacent spans with identical styles are two fragments to Blink, and
   Parley cannot see the boundary.** `Line::items` splits on resolved style index,
   so it hands both spans over as one glyph run. Not modellable from Parley's side;
   designed out of generated cases instead (see below), and reported rather than
   absorbed when it does occur.

## What changed

- **The golden schema is fragment-structured.** `GlyphOutput` is now a list of
  `Fragment { origin_x, origin_y, style, glyphs }` with glyph offsets *local* to the
  origin, rather than a flat list of absolute positions. `GlyphOutput::glyphs()`
  derives the flat absolute view for the comparison and the report generator. The
  text format gains a `fragment` header line; all 55 goldens were re-recorded.
- **The comparison tolerance is the sum of two half-ULPs.** `skp_parser` serialises
  the fragment origin and the glyph's offset as *separate* 6-significant-figure
  numbers, so the floor on an absolute position is
  `half_ulp_6sig(origin) + half_ulp_6sig(offset)`, not `half_ulp_6sig(origin +
  offset)`. While a line could hold only one fragment every origin was 0 and the two
  agreed; storing only the sum threw away what the new floor needs. This alone
  accounted for 39 of 60 apparent failures in the first measurement.
- **Fragment origins are f64 on both sides.** An origin is an accumulator — Blink
  sums snapped widths across a line, Parley sums advances in f64 for the same reason
  already documented in Phase 1, and Chrome's is a decimal this crate must not
  re-round. Narrowing it to f32 costs half an f32 ULP, about 8e-6 px at corpus
  widths, which is a sixth of the serialisation floor and was on its own enough to
  fail an otherwise-matching case (seed 216, 6.1e-5 against a 5.5e-5 floor). Glyph
  offsets stay f32: they are single shaped values, f32-native on both sides.
- **`compare` gained a `Fragmentation` variant.** Checked after glyph count and
  before any position: when the two sides split the same glyphs differently, every
  position after the divergence follows from that one cause, so it is reported once
  instead of as a wall of diffs. This is also how rule 3 surfaces as a diagnosis
  rather than as silent wrongness. `FailureSignature` gained a matching variant so
  the minimiser can't slide between a structural and a positional failure.
- **`MAX_RUNS` is 4 again**, and `sample_font_size` now refuses a size equal to the
  previous run's, which is what keeps rule 3 out of generated cases. Font size is
  the only property whose inequality is enough to guarantee the two spans resolve to
  different styles.

## Result

Over a 150-seed survey against live Chrome, **109 of 114 multi-span cases pass
exactly** — no tolerance widening anywhere, just the serialisation floor. The five
failures are checked in under `known_failing/` with notes: three are line-breaking
divergences (the two sides put different glyph counts on a line), one is a baseline
divergence of exactly 1px, and one is a genuine open question about rule 2 — see
below.

The corpus is now 50 `generated/` (30 single-span, 20 multi-span across 2, 3 and 4
spans) and 5 `known_failing/`.

## The Phase 5 `known_failing/` corpus was misattributed

All 15 cases that
[`…-phase5.md`](./glyph-positioning-chrome-parity-phase5.md) checked in as
decomposition-cluster-bug repros are in fact this snapping, at a wrapped line's
hanging trailing whitespace. Every one of them now passes and has been promoted into
`generated/`, with its note corrected.

That doc's "The decomposition-cluster bug is not rare" section — which read the same
~1/64 px residuals as evidence of a `parley_engine` cluster bug dominating ~86% of
generated cases — should be read with that in mind. Its controlled check (a
plain-ASCII multi-line case that matched Chrome perfectly) does not discriminate
between the two hypotheses, because what decides whether this snapping fires is
whether a line has hanging trailing whitespace, not whether the text is ASCII.

Phase 4's separate finding — that letter and word spacing are double-charged for a
font-decomposed base followed by an explicit combining mark, with `dx` numerically
equal to the run's spacing value — is **not** affected and remains a real
`parley_engine` bug. `LETTER_SPACING_RANGE_PX` and `WORD_SPACING_RANGE_PX` stay
pinned to zero for that reason, unchanged by this work. What is withdrawn is only
the claim that a *spacing-free* variant of it dominates the corpus.

## Open: rule 2's exact condition

`known_failing/seed_0303.txt` is the one case that challenges the model rather than
the line breaker. Chrome hung the final line's trailing whitespace as its own
fragment; the model did not, because the break landed on a span boundary.

Rule 2's "only when the break splits a span" half came from a case where the break
was on a span boundary *mid-text* and Chrome did not hang. In seed 303 the break is
at the *end of the text*, and Chrome hangs anyway. So "splits a span" and "is the
end of the text" are confounded in the evidence gathered so far, and the correct
condition needs a controlled experiment separating them — cases holding one constant
while varying the other — not another guess from a mixed sample.

## Diagnosed: seed 434's line-breaking divergence

`known_failing/seed_0434.txt`'s note used to just say "Parley put 4 glyphs where
Chrome put 6" without a cause. It has one now: the third run's text contains `¤`
(U+00A4 CURRENCY SIGN, Unicode line-break class `PR`, prefix numeric) immediately
followed by `(` (class `OP`, open punctuation), with no digit anywhere nearby. Real
Chrome treats `PR` immediately before `OP` as a line-break opportunity there; Parley's
line breaker (`parley_engine`'s `icu_segmenter` 2.2.0, dictionary-mode `LineSegmenter`)
does not, so the two sides disagree about where the line can wrap, and everything after
the divergence follows from that.

This was confirmed against live container Chrome
(`parley_tests/glyph_positioning_recorder/container/run.sh`) on isolated single-run
variants of the run-2 substring, not just the original 3-run case:

| text | container width | result |
| --- | --- | --- |
| `С¤(Ựâ` (original) | 60px | breaks between `¤` and `(` |
| `С¤Ұâ` (`(` removed) | 60px | no break — stays on one line |
| `С(Ựâ` (`¤` removed) | 60px | no break — stays on one line |
| `С¤(1â` (digit right after `(`) | 60px | still breaks between `¤` and `(` |
| `С(¤Ựâ` (`(` before `¤`) | 60px | no break — stays on one line |

The third row rules out the obvious alternative explanation: UAX #14's LB25 has a
regex-based numeric-context exception (`(PR|PO)? (OP|HY)? IS? NU …`) that keeps a
prefix symbol, an opening paren/hyphen, and a following *number* glued together (e.g.
`$(-1,234.56)`). If that were what Chrome was applying, putting a digit right after the
`(` should be the one case *most* likely to suppress the break — instead it breaks in
exactly the same place regardless, meaning Chrome isn't invoking the numeric-context
rule here at all. It reads `PR` immediately before `OP` as a plain break opportunity,
unconditionally.

Directly querying `icu_segmenter` 2.2.0's `LineSegmenter` (dictionary mode, the same
constructor `parley_engine::analysis` uses for `WordBreak::Normal`) on `"С¤(Ựâ"` in
isolation returns no break opportunity anywhere in the string. The pair-table logic
that decides this is data-driven (compiled into `icu_segmenter_data`, not readable
Rust), so the exact rule revision it was generated from wasn't checked — but the
behavior itself matches the *older*, context-free reading of LB25 (PR/PO unconditionally
glued to a following OP), predating the regex rewrite real ICU's line-break rules use.
That reads as an `icu_segmenter`-side version/rule-revision gap, not a `parley_engine`
bug — worth filing upstream against `icu_segmenter`/`icu4x`, but not something to chase
in this repo. Left in `known_failing/` as diagnosed-but-open.

## Bidi

Structurally the model is bidi-safe: Blink accumulates fragment origins in **visual**
order after bidi reordering, and Parley's `Line::items` is in visual order too, so
the recurrence applies in the same order on both sides with no change.

**None of it has been tested under bidi, and cannot be today.** The bundled
Roboto-Regular has 0 codepoints in the Hebrew, Arabic, Syriac, Devanagari or Thai
blocks (75 Greek, 255 Cyrillic, all LTR), and `Case` generation excludes the
RTL/complex blocks outright. Validating this under bidi needs a bidi-capable face
added to `FONTS` first. Two risks to settle when that happens:

- Index-by-index glyph pairing is what `compare` relies on, and Phase 4's own comment
  flags it as unproven under bidi. Nothing here improves that.
- Rule 1's span merge must merge only within a bidi run. Blink splits inline items at
  bidi-level boundaries and so does Parley, so they should agree — but if they don't,
  it now surfaces as a `Fragmentation` mismatch rather than as a wall of positions,
  which is the point of having that variant.
