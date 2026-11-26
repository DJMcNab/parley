# Noto Global Fonts

These are Google's Noto Fonts, used to test Parley's support for glyphs not covered by Roboto and Arimo.
Both are licensed under the OFL 1.1:

- `NotoKufiArabic-Regular.otf`'s license is in `NotoKufiArabic-Regular-LICENSE.txt`.
- `NotoSansCJKsc-Regular-subset.otf`'s license is in `NotoSansCJKsc-Regular-subset-LICENSE.txt`.

## Noto Sans CJK Simplified Chinese Subset

For our tests, we use only a subset of Noto Sans CJK, as the full CJK font is more than 15MiB, which is impractically large for storing in-repo.
The full unsubsetted font was obtained from <https://github.com/notofonts/noto-cjk/releases/tag/Sans2.004> (specifically,
[Language Specific OTFs Simplified Chinese (简体中文)](https://github.com/notofonts/noto-cjk/releases/download/Sans2.004/08_NotoSansCJKsc.zip)).
The original font file (`NotoSansCJKsc-Regular.otf`), had sha256 hash `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b`.

Currently, this subset contains two ideographs (骨 U+9AA8 and 水 U+6C34) and the fullwidth punctuation needed to test `text-spacing-trim`:
ideographic space, comma and full stop (U+3000–U+3002), corner brackets (U+300C, U+300D), the katakana middle dot (U+30FB), fullwidth
parentheses, comma, full stop, colon and semicolon (U+FF08, U+FF09, U+FF0C, U+FF0E, U+FF1A, U+FF1B), and the curly quotes (U+2018, U+2019,
U+201C, U+201D), which this font draws fullwidth.
The subset keeps the font's `halt` and `vhal` GPOS features for these glyphs.
It was produced using [fonttools](https://github.com/fonttools/fonttools) (version 4.62.1):

```sh
fonttools subset NotoSansCJKsc-Regular.otf \
  --output-file=NotoSansCJKsc-Regular-subset.otf \
  --unicodes=U+9AA8,U+6C34,U+3000,U+3001,U+3002,U+300C,U+300D,U+30FB,U+FF08,U+FF09,U+FF0C,U+FF0E,U+FF1A,U+FF1B,U+2018,U+2019,U+201C,U+201D \
  --layout-features='*' \
  --glyph-names \
  --symbol-cmap \
  --legacy-cmap \
  --notdef-glyph --notdef-outline \
  --recommended-glyphs \
  --name-IDs='*' \
  --no-hinting \
  --desubroutinize
```

If in adding future tests you need more glyphs, then replacing the current subset is fine, so long as this doc is updated.
Note that this might change if the font file gets to be a considerable size; recondier replacing it if larger than, say, 150KiB.
