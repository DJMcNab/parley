# Phase 5 — CI-facing test + initial golden dataset

Implementation record for Phase 5 of
[`glyph-positioning-chrome-parity.md`](./glyph-positioning-chrome-parity.md).
Read the parent plan first, then
[`…-phase4.md`](./glyph-positioning-chrome-parity-phase4.md) for the state
Phase 4 left the pipeline in — this file starts from there.

**Status: done.** A grilling session resolved the open decisions the earlier stub
left, and running the pipeline end to end for the first time at real corpus scale
surfaced one more: the decomposition-cluster bug (§ below) turned out to be far more
common than Phase 4's fuzzing had suggested, which changed the initial corpus's shape
and size.

## What landed

- **B8** (`parley/src/layout/data.rs`, `parley/src/style/mod.rs`) is landed: the two
  affected PNG snapshot tests were re-accepted, and `cargo test --workspace` is green.
- **`tests/glyph_positioning/`** has four directories, not three:
  `handwritten/`, `regressions/`, `generated/`, and **`known_failing/`** (new). The
  first three are expected to pass; `known_failing/` holds checked-in repros of real,
  tracked bugs and is expected to keep failing — see "The test" below.
- **`parley_tests/tests/glyph_positioning.rs`** is a single `#[test]`
  (`glyph_positioning_matches_chrome`), registered in `tests/mod.rs`. It has no Docker
  or network dependency: every golden file already carries both the `Case` and
  Chrome's recorded output.
- **`regenerate_goldens`** no longer auto-generates new `generated/` seeds. It now
  only re-records whatever's already on disk in any of the four directories. See
  "Why `regenerate_goldens` shrank" below for why the original
  `Case::from_seed(0..GENERATED_SEED_COUNT)` auto-fill was removed.

## Decisions from grilling

- **`known_failing/` replaces an in-code seed list.** The initial plan was a
  `KNOWN_FAILING_SEEDS: &[u64]` constant in the test file. Instead, a case that hits a
  known bug is promoted to its own golden file in `known_failing/`, with a `note`
  explaining which bug. This makes each entry self-documenting and lets
  `regenerate_goldens`'s existing "re-record whatever's on disk, preserving `note`"
  rule apply uniformly — no separate exclusion mechanism to keep in sync.
- **`known_failing/` cases assert failure, not skip.** The test calls `compare()` on
  every `known_failing/` case and asserts it still returns `Err`. If a fix lands and
  the case starts passing, the test **fails**, telling you to promote the file out of
  `known_failing/` — rather than the fix going unnoticed because nothing was
  asserting either way.
- **One aggregate `#[test]`, not one per file.** It runs every case in all four
  directories regardless of earlier failures and reports every unexpected result
  together at the end, so a CI run surfaces every broken case at once.
- **`handwritten/` and `regressions/` stay empty for now.** Nothing from Phase 4's
  bring-up (B9's hanging-space case, B10's combining-mark case) was promoted into
  permanent fixtures this phase; that can happen later if wanted.

## The decomposition-cluster bug is not rare

Phase 4's fuzzing (with `LETTER_SPACING_RANGE_PX`/`WORD_SPACING_RANGE_PX` pinned to
`(0.0, 0.0)` and `MAX_RUNS = 1`) found the bug described in its "Findings" section —
a character HarfBuzz shapes as multiple glyphs under one cluster, which desyncs
`parley_engine`'s cumulative advance from Chrome's — but only saw it in one case out
of a 60-case sample, so the Phase 5 stub treated it as a rare, not-yet-localized edge
case alongside line-breaking divergences and a B13 residual.

Running `Case::from_seed(0..50)` at real scale showed otherwise: **43 of 50 cases
failed**, and a controlled check (a plain ASCII, multi-line, no-spacing case compared
byte-for-byte against Chrome) passed perfectly, ruling out wrapping or B13 as the
cause. Dumping per-glyph diffs for a sample of the failures showed the residual
scattered across many glyphs at varying sign and magnitude (not concentrated at line
or run boundaries), and the affected cases' text was checked directly:
[U+1EAB `ẫ`-style double-diacritic characters](https://en.wikipedia.org/wiki/Combining_character)
and other canonically-decomposing letters (Vietnamese, Slavic, Baltic diacritics) are
common in the sampling alphabet, and a string of realistic length (40–120 chars)
contains several of them purely by chance. Each occurrence perturbs the cumulative
advance for the rest of that line, so the effect compounds across a line with several
such characters — this is the same bug the Phase 4 doc found via the letter/
word-spacing workaround, just manifesting without needing any spacing at all, and far
more often than one anecdotal case suggested.

(A speculative fix modelling B13's `LayoutUnit` ceil-rounding at wrapped-line
boundaries was tried first, on a misreading of a truncated diff sample, and made no
measurable difference — it was reverted rather than left in with an inaccurate
rationale comment.)

Given the scale, restricting the sampling alphabet or fixing the underlying
`parley_engine` cluster-boundary bug were both out of scope for this phase (see
"Open follow-ups"). The initial corpus was curated around the bug instead.

## The initial corpus is 15 passing + 15 known-failing, not 50

Reaching Phase 4's originally-planned 50 clean `generated/` cases would mean
searching roughly 350 candidate seeds at the ~86% failure rate above. Rather than do
that (or check in all ~300 failing candidates along the way), the initial corpus is
smaller and the failing side is a representative sample:

- `generated/`: 15 cases, `Case::from_seed(n)` for the first 15 seeds (starting at 0)
  that pass `compare()` against Parley.
- `known_failing/`: 15 cases, the first 15 seeds (starting at 0) that fail, each
  classified and noted as either the decomposition-cluster bug or a line-breaking
  divergence (glyph-count mismatch or a `dy` off by roughly a line height — the other
  known issue from Phase 4's fuzzing). In this run, every one of the first 15 failing
  seeds was the decomposition-cluster bug; no line-breaking-divergence example landed
  in the sample by chance.
- Seeds between the smallest and largest kept are not all represented — most were
  tried, failed on the decomposition bug, and simply not persisted (matching the
  chosen tradeoff of a small representative failing sample over exhaustive capture).

This was produced by a one-off scratch tool (not checked in) that tried seeds in
order, capturing and comparing each one live against the recorder container, and
wrote the first 15 of each outcome to disk. `GENERATED_SEED_COUNT` no longer exists as
a driving constant — see below.

## Why `regenerate_goldens` shrank

The original design auto-generated `generated/`'s seeds from a fixed range
(`Case::from_seed(0..GENERATED_SEED_COUNT)`) every run, skipping any seed already
claimed by `known_failing/`. That's unsound at this failure rate: `known_failing/`
only contains a curated *sample* of failing seeds, not every failing seed in any
range, so "not in `known_failing/`" does not imply "passes." Running that logic with
`GENERATED_SEED_COUNT` raised to backfill after curation in fact added several
untested — and, on inspection, failing — seeds straight into `generated/`, since nothing
about the auto-fill loop compares against Parley (`regenerate_goldens` deliberately
never does; that's `cargo test`'s job).

`regenerate_goldens` now only re-records whatever golden files already exist,
preserving `seed` and `note` — the same rule it always applied to `handwritten/` and
`regressions/`, now applied uniformly to `generated/` and `known_failing/` too.
Growing `generated/` beyond its current 15 needs an explicit pass/fail search (as
above), not a blind seed range.

## Open follow-ups (not done this phase)

- **Root-cause and fix the decomposition-cluster bug** in
  `parley_engine/src/shape/shaped_text.rs`'s cluster-boundary detection (see the
  Phase 4 doc's "A real Parley bug, found via the uniform offset cases"), or restrict
  the sampling alphabet to avoid it. Either would let `generated/` grow past its
  current 86%-failure ceiling.
- **Grow `generated/` and `known_failing/`** once one of the above lands, and
  consider promoting a `handwritten/` case or two from Phase 4's bring-up fixtures.
- **B13's line-wrap-boundary residual** is still unconfirmed at real scale — the
  decomposition bug dominates every sample taken so far, so B13 has not been cleanly
  observed in isolation since Phase 4's small-sample "B12/B13 preview." Revisit once
  the decomposition bug no longer swamps the signal.

## Verification

- `cargo test -p parley_tests glyph_positioning_matches_chrome` passes with no
  Docker/network access.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets --all-features`
  are clean.
- `regenerate_goldens` was re-run against the live container and reproduced
  byte-identical files (idempotency confirmed).
