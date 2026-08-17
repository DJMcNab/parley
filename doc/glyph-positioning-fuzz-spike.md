# Glyph-positioning fuzz classification spike

This is an exploratory report, not a proposal to add permanent test
classification infrastructure.

## Survey

Three local Chrome-recorder fuzz passes covered 1,000 deterministic cases:

- seeds `3000000..3000099` (100 cases),
- seeds `3000100..3000499` (400 cases), and
- seeds `3000500..3000999` (500 cases).

There were 47 mismatches and no recorder/harness failures. The first five were
all minimised; later batches minimised only a new observable shape or the
deterministic one-in-five review sample for an existing shape. The second
selective batch minimised seven more cases, requiring 13,048 Chrome captures
and trying 18,232 candidates. This confirms that minimising every raw fuzz
failure is unnecessarily expensive for triage.

## Observed failure families

The labels below describe the comparison result, not a proven product root
cause.

| Observable family | Cases | Representative | What it shows |
| --- | ---: | --- | --- |
| Combining-mark glyph-count divergence | 16 | `3000726` | Chrome emits one more glyph than Parley. Every minimised sample retains U+0300, including the three cases that did not look like this before minimisation. |
| Sub-layout-unit x residual | 13 | `3000626` | One or two horizontal positions exceed their own tolerance narrowly (for example `0.000553px` vs `0.000550px`), with zero y difference. |
| Fragmentation divergence | 15 | `3000589` | The glyph sequence agrees, but Chrome and Parley split it into different fragments. The reduced cases include adjacent same-style runs and single-run whitespace boundaries. |
| Horizontal offset | 2 | `3000185` | A material horizontal-only discrepancy. The smallest example is `΄ʼ T` at 16px, where the final glyph is displaced by `+0.312512px`. |
| Vertical offset | 1 | `3000649` | A single `!` at 16px has an exact `-1px` Parley-versus-Chrome baseline difference. This is the only remaining unclassified comparison shape. |

The first three are recurring families, while the last two need a focused
investigation before they should be named after a root cause. In particular,
the fragmentation class can arise from differing line breaks, hanging
whitespace, or run/fragment boundary decisions; it should not yet be treated
as one bug.

## Exploratory classifier

`parley_tests/glyph_positioning_recorder/src/bin/classify_spike.rs` is a
recorder-only, intentionally disposable triage tool. It reads the fuzz output
directory and prefers `minimised.txt`/`minimised_mismatch.txt` when available;
otherwise it classifies raw `case.txt`/`mismatch.txt` artifacts.

Run it after a fuzz pass:

```sh
cargo run -p parley_glyph_positioning_recorder --bin classify_spike -- \
  --out target/glyph_positioning_exploration
```

It writes `classification_spike_report.txt` alongside the artifacts. For each
class, it marks the first case and every fifth later case as `REVIEW`. A raw
case is marked `MINIMISE` only when it is either unclassified or is such a
review sample. Feed only those paths to the existing minimiser, then rerun the
classifier:

```sh
parley_tests/glyph_positioning_recorder/container/run.sh minimise \
  --out target/glyph_positioning_exploration \
  target/glyph_positioning_exploration/seed_3000649/case.txt
```

This provides the requested one-in-five spot-check cadence without turning a
fast fuzz pass into thousands of unnecessary browser captures.

## If this is developed further

- Keep the tool in the recorder crate and its artifacts under `target/`; do not
  make these mismatch-text labels part of the golden-file format or public API.
- Give the `-1px` vertical case a controlled baseline/line-height experiment.
- Split fragmentation only after controlled experiments distinguish hanging
  whitespace, wrap boundaries, and run-boundary fragmentation.
- Investigate the U+0300 count divergence and the material horizontal-offset
  case independently; their current labels deliberately avoid guessing at the
  responsible layout subsystem.
- Promote a minimised example to `known_failing/` only after a root cause and
  ownership are established.
