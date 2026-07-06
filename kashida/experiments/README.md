# Kashida experiments

Standalone experiment crates for the Kashida-justification investigation. Each
crate has its own empty `[workspace]` table so it is **not** a member of the
Parley workspace and can be built independently:

```sh
cd harfrust_probe && cargo run
```

## `harfrust_probe`

Shapes Arabic strings with HarfRust 0.10 directly and prints, per glyph:
glyph id, source cluster, advances/offsets, and the glyph flags
(`UNSAFE_TO_BREAK`, `UNSAFE_TO_CONCAT`, `SAFE_TO_INSERT_TATWEEL`). It also uses
Skrifa to look up the U+0640 TATWEEL glyph id and advance, and compares baseline
shaping of `كتاب` against re-shaping with explicit tatweels inserted
(`كـتاب`, `كــتاب`).

See `RESULTS.md` for captured output and analysis.

## Fonts & licensing

**Constraint:** fonts may only be downloaded from Google Fonts and only if
Apache-2.0 licensed.

**Finding (verified via the `google/fonts` GitHub repo):** there is **no
Apache-2.0-licensed Arabic-script font on Google Fonts**. The `apache/`
directory contains 44 families, none supporting Arabic. All Noto Arabic fonts
(Noto Naskh/Sans/Kufi Arabic) are now SIL OFL (under `ofl/`). The historically
Apache-2.0 "Droid Arabic Naskh/Kufi" have been removed from the repo entirely
(404). So **nothing was downloaded.**

For local-only experimentation the probe references, *by path*, the
`NotoKufiArabic-Regular.otf` already vendored in this repository at
`parley_dev/assets/fonts/noto_fonts/` (SIL OFL 1.1, "Copyright 2022 The Noto
Project Authors"). It is **not copied into this folder and not redistributed** —
the experiment only reads it in place. If you need a from-scratch reproduction
without the vendored asset, point the probe at any Arabic OTF/TTF:
`cargo run -- /path/to/arabic.ttf`.
