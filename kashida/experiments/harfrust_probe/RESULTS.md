# `harfrust_probe` results

Captured with HarfRust 0.10.0 + Skrifa 0.43.2, font = vendored
`NotoKufiArabic-Regular.otf` (OFL; units_per_em = 1000).

## Skrifa: U+0640 TATWEEL
```
cmap(U+0640)           = GlyphId(114)
advance_width(tatweel) = 171 font units
```
Skrifa can resolve the tatweel glyph id (via `charmap().map`) and its advance
(via `glyph_metrics().advance_width`) — everything needed to synthesize a
tatweel glyph outside the shaper.

## `SAFE_TO_INSERT_TATWEEL` is produced only when requested

`كتاب` shaped WITHOUT `BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL`: no glyph
carries the tatweel flag. Shaped WITH the flag, the interior cursive-join
clusters carry `SAFE_TO_INSERT_TATWEEL`:

```
vis gid cluster   x_adv   flags
  0 309       3       0    -
  1  13       3     778    -
  2   9       2     316    SAFE_TO_INSERT_TATWEEL
  3 282       1       0    UNSAFE_TO_BREAK|SAFE_TO_INSERT_TATWEEL
  4  17       1     391    UNSAFE_TO_BREAK|SAFE_TO_INSERT_TATWEEL
  5  63       0     576    -
```

The flag is per-glyph; it is set on the glyph *before which* (in logical order)
a tatweel may be inserted. Word-final/word-initial positions (clusters 0 and 3
here) are correctly NOT flagged.

## Naive post-shaping tatweel insertion == re-shaping (for this font)

Baseline `كتاب` (4 chars) vs. re-shaping `كـتاب` (tatweel inserted after kaf)
and `كــتاب` (two tatweels):

| string        | glyph ids (visual)             | total x_adv |
|---------------|--------------------------------|-------------|
| `كتاب`        | 309,13, 9,282,17, 63           | 2061        |
| `كـتاب`       | 309,13, 9,282,17, **114**,63   | 2232 (+171) |
| `كــتاب`      | 309,13, 9,282,17, **114,114**,63 | 2403 (+342) |

**Every base-letter glyph id and advance is identical across the three**; the
only difference is one/two extra tatweel glyphs (id 114, advance 171) spliced in
at the flagged join. i.e. for a font whose kashida is a plain repeatable tatweel
(Noto Kufi), inserting `n` copies of the Skrifa-resolved tatweel glyph at a
`SAFE_TO_INSERT_TATWEEL` position reproduces the re-shaped result exactly, with
elongation linear in `n` (n × 171 units). **No re-shaping is required.**

### Caveat
This equivalence is exactly what the flag *guarantees for the base text*: the
surrounding letters won't change. It does **not** guarantee the tatweel *looks*
right — fonts with curved/calligraphic kashida, `jalt` alternates, or an MSHQ
variation axis (e.g. Amiri, Gulzar) want either a font-specific extender glyph
or a variable-axis stretch, not a flat U+0640. For those, high-quality output
still needs font-aware handling or re-shaping. The flag tells you *where* it is
safe to elongate, never *how much* nor *how pretty*.
