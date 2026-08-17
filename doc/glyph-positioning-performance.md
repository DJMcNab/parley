# Glyph-positioning recorder performance

Investigation performed on 2026-08-17 using the pinned recorder image and the four
checked-in known failures. The existing `parley-recorder` container was left running;
benchmarks used `parley-recorder-speed-1` on host ports 9525/9526.

## Result

Minimisation now accepts `--jobs N`. Each worker owns a long-lived Chrome instance in
the same container and takes the next input from a shared queue. Captures are isolated
under `/skp/capture-<pid>-<counter>/`, removing the old global clear/capture/list race.

| workers | inputs | Chrome captures | wall time | captures/s | relative throughput |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1 | 2,156 | 146.09s | 14.76 | 1.00x |
| 2 | 2 | 3,352 | 151.83s | 22.08 | 1.50x |
| 4 | 4 | 4,302 | 145.55s | 29.56 | 2.00x |

The input sets differ because the checked-in corpus has only four known failures, so
captures/s is the useful scaling measure rather than raw batch time. Seed 303 reduced
to byte-equivalent canonical content in the one-, two-, and four-worker runs. No SKP,
WebDriver, or agent failures occurred in the parallel runs.

Four workers are a reasonable starting point on this machine. Scaling is sub-linear:
the Chrome instances and concurrent `skp_parser` processes contend for CPU, and each
worker has its own candidate cache. The default remains one worker to avoid surprising
memory use; unattended batches should opt in with `minimise --jobs 4`.

## Capture freshness

Chromium source inspection found that `printToSkPicture` serializes the current cc
layer tree directly (`content/renderer/gpu_benchmarking_extension.cc`), while an rAF
callback itself runs before its frame paints. This explains why zero or one plain rAF
produced an empty SKP and why the old nested-rAF barrier worked.

Queueing `setTimeout(0)` from the first rAF moves capture to a task after that frame's
paint, removing the second display interval without weakening the barrier. On seed 303:

| barrier / scheduling | wall time | result |
| --- | ---: | --- |
| two rAF, normal pacing | 146.09s | baseline |
| two rAF, `--disable-frame-rate-limit` | 130.65s | exact same reduction, high CPU |
| explicit CDP `HeadlessExperimental.beginFrame` | 143.05s | exact same reduction |
| rAF then task, uncapped | 114.52s | exact same reduction |
| rAF then task, normal pacing | 114.97s | exact same reduction |

The frame-rate flag adds no material value with the after-paint task and is not kept.
Explicit begin-frame control is sound, but its ChromeDriver/CDP round trip costs nearly
as much as normal pacing and adds considerably more machinery.

## Character probes

The old character pass scanned every lower codepoint (up to a 600-character alphabet
prefix) at every surviving text position. It now groups candidates by Unicode Script
and General_Category. Up to the existing 600-probe safety bound is spent within the
original character's group; every foreign group contributes only its lowest
representative. Thus a Latin lowercase original can still expose a glyph-specific issue
anywhere in Latin lowercase, while a failed `a` probe skips `b` through `z` when the
original belongs elsewhere. The grouping is Unicode data, not a table for the currently
bundled Roboto font.

For seed 303 this reduced candidates from 2,906 to 1,448, captures from 2,156 to 1,064,
and wall time from 146.09s to 56.52s when combined with the after-paint barrier. The
canonical text changes under this intentionally coarser heuristic, but the failure
signature is preserved.

The combined four-worker run over all checked-in known failures preserved all four
signatures and completed in 59.38s (3,059 captures), down from 145.55s (4,302 captures)
for parallelism alone. A final cold launcher run took 69.73s including container startup
and a small incremental compile, with the same outputs and counts.

## Further opportunities

- Measure `skp_parser` process startup and consider a persistent in-container parser.
- Share completed and in-flight candidate captures across minimiser workers. The
  current per-worker caches preserve simple ownership but may repeat work when seeds
  converge.

## Follow-up improvements

The minimiser now uses an output-only capture path. It still fetches the parser's JSON
commands, which are required to reconstruct glyph positions, but does not transfer the
raw `.skp` bytes for every candidate. Raw SKPs remain available to `fuzz_loop`, where
they are retained as mismatch diagnostics. A repeat of seed 303 took 54.49s versus
56.52s before this change (1,064 captures in both runs); the transfer saving is modest
because parsing and the frame barrier dominate.

Each minimisation outcome records candidates, Chrome captures, and cache hits for the
baseline and every reduction phase. The batch report aggregates those counters, making
the expensive pass visible before further heuristic changes are attempted. For seed
303, character scanning accounted for 725 of 1,064 Chrome captures; character ddmin was
next at 184. The scalar ladder tried 287 candidates but served 160 from cache.

Discovery-mode minimisation is resumable. Seeds with both output artifacts already
present are loaded into the new batch report without opening Chrome; partial artifacts
are rerun, explicit paths are always rerun, and `--force` opts into rebuilding the full
discovered batch.

`fuzz_loop` now accepts `--jobs N`. Long-lived Chrome sessions claim seeds from one
atomic counter, so a finite run performs exactly `--max-cases` attempts without
duplicates. A failure in one worker stops the pool, and Ctrl-C allows captures already
in flight to finish before sessions close. On the same fixed 100-seed range, one worker
ran at 14.30 cases/s and four ran at 25.77 cases/s (1.80x); both found the same five
mismatches with no harness failures.
