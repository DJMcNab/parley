# Glyph-positioning report generator

`parley_glyph_positioning_report` turns a glyph-positioning golden into a page you
can look at, for diagnosing *why* a case disagrees with Chrome. It is a developer
tool: nothing in CI depends on its output.

```sh
cargo run -p parley_glyph_positioning_report                    # the checked-in corpus
cargo run -p parley_glyph_positioning_report -- PATH... [--out DIR]
```

Paths may be golden files or directories, walked recursively, so a fuzz haul's
`seed_*/case.txt` artifacts work as-is. Output defaults to
`target/glyph_positioning_report/`, mirroring the input's directory structure, with
an `index.html` at the root.

## What a case page shows

The four renderings form a 2×2 grid — the two sides above what they add up to:

| | |
| --- | --- |
| **Chrome (recorded)** — the golden's glyphs, rasterised. | **Parley (now)** — what Parley produces today, with a pastel box per cluster *behind* the glyphs. |
| **Overlay** — the two tinted red/blue and multiplied: agreement reads near-black, disagreement as colour fringing. Most real differences are sub-pixel, so zoom. | **Live DOM** — the recorder's exact DOM and CSS, rendered by the reader's browser. Labelled as indicative: it is not the pinned Chrome, which runs with `--font-render-hinting=none`. |

Below them:

- **Characters** — one entry per `char` (`U+xxxx` plus the character itself),
  background-tinted by the Parley cluster owning it.
- **Glyphs** — the per-glyph diff table, with each glyph attributed to its cluster.

Hovering any cluster box, character or table row highlights that cluster in all
three. Parley is re-run at report time, so a `known_failing/` case that has started
passing shows up as such rather than being taken on trust.

## Constraints worth knowing before changing it

- **Both sides rasterise through one function** (`raster::render`, fed a
  `GlyphOutput`). Chrome's side has no `Layout`, and drawing Parley's from
  `positioned_glyphs` would use an f32 running offset rather than the f64-accumulated
  numbers in the diff table — the image could then contradict the table at exactly
  the magnitudes being investigated. Hinting is off, matching the recording.
- **Cluster boxes are Parley-only.** Cluster geometry is Parley's; Chrome's side
  carries no cluster information.
- **Pages are self-contained except for fonts.** Rasters are inlined; font bytes are
  fetched from `SupportedFont::url` and verified against `SupportedFont::sha256`
  before a `FontFace` is constructed, since `@font-face` cannot carry a
  subresource-integrity hash. A face that fails to arrive replaces the live-DOM panel
  with an error rather than silently rendering in a fallback face.
- **The index is not self-contained** — it references each case's raster PNGs, so
  rasters are written to disk as well as inlined.
- **No CSS `mask-image`.** Mask images are CORS-restricted and every sibling file on a
  `file://` page is a foreign origin, which silently blanks the index's thumbnails.
  Tinting therefore uses `<img>` plus a same-document SVG `feColorMatrix` filter.
- **The 2×2 grid is flex-wrap, not `grid`.** A cell is as wide as its case's raster and
  cannot be scaled (the cluster boxes are in layout coordinates), so the two-per-row
  limit comes from a generated `max-width` on the container. `repeat(auto-fit, …)`
  cannot express this — it needs a *fixed* track size. A case too wide for two columns
  wraps to one.
- **Cluster colours are built from `--hue` on each element.** A shared `--pastel`
  custom property on `:root` resolves against the root's `--hue`, making every
  cluster the same colour.
- The index sections by **source directory**, ordered ad-hoc/fuzz → `known_failing/` →
  the pass-expected directories, with anything contradicting its directory hoisted to
  the top. Grouping by *bug* is the obvious improvement, and is deliberately deferred
  until the failure minimiser emits a classification worth grouping on.
