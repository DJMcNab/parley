# Glyph-positioning snapshot tests

The glyph-positioning test compares Parley's current output with Chromium output
stored in `parley_tests/tests/glyph_positioning`. It uses only checked-in data and
does not require Docker, Chromium, or network access.

Run it with:

```shell
cargo test -p parley_tests --test tests glyph_positioning
```

The corpus is split by purpose:

- `generated`, `handwritten`, and `regressions` contain cases that must pass.
- `known_failing` contains tracked bugs that must continue to fail. Each file should
  have a `note` identifying the bug.

All cases run even if an earlier one fails, so one test run reports every unexpected
result. If a case in `known_failing` starts passing, move it to `regressions`. A
failure elsewhere is either a Parley regression or a deliberate output change that
needs review.

Each snapshot contains the complete input case as well as Chromium's output. The seed
records provenance only; tests never regenerate a case from it. This keeps existing
snapshots stable when the generator changes.
