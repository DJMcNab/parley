// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Everything a page needs to say about one case, derived once here so the HTML
//! generators are pure formatting.
//!
//! The verdict is re-derived rather than read off the golden's directory: the whole
//! point of the report is to show what Parley does *now*, including a `known_failing/`
//! case that has quietly started passing.

use std::collections::HashMap;
use std::ops::Range;

use parley::{Layout, LayoutContext, PositionedLayoutItem};
use parley_glyph_positioning_cases::{
    FONTS, FailureSignature, GlyphOutput, Golden, Mismatch, PositionedGlyph, compare,
    half_ulp_6sig, x_matches, y_matches,
};
use parley_glyph_positioning_extract::{layout, parley_output};

/// Padding, in CSS px, around the layout box in every raster.
///
/// Glyphs routinely paint outside their line box — tall diacritics above it, descenders
/// and overshoot below — and the live-DOM panel gets the same padding, so all four
/// panels stay in one coordinate space. Cluster boxes are positioned in that same
/// padded space.
pub(crate) const PADDING: f32 = 12.0;

/// How much bigger than a CSS px the rasters are.
///
/// Rasters are emitted at their CSS-px width, so this is pure detail-on-zoom: browser
/// zoom reveals real subpixel structure rather than blur, which is the scale most of
/// these disagreements live at.
pub(crate) const SCALE: f32 = 4.0;

/// One case, fully analysed.
#[derive(Debug)]
pub(crate) struct CaseReport {
    /// The case and Chrome's recorded output.
    pub(crate) golden: Golden,
    /// What Parley produces for the case today.
    pub(crate) parley: GlyphOutput,
    /// How Parley's output differs from Chrome's, or `None` if it matches.
    pub(crate) mismatch: Option<Mismatch>,
    /// One row per glyph, in emission order, with both sides where they exist.
    pub(crate) rows: Vec<GlyphRow>,
    /// Parley's clusters, in emission order.
    pub(crate) clusters: Vec<ClusterBox>,
    /// One entry per `char` of the case's text, in logical order.
    pub(crate) chars: Vec<CharEntry>,
    /// The shared canvas geometry for every panel.
    pub(crate) geometry: Geometry,
}

/// The canvas every panel of a case shares, in CSS px, including [`PADDING`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Geometry {
    /// Total width, including padding on both sides.
    pub(crate) width: f32,
    /// Total height, including padding on both sides.
    pub(crate) height: f32,
}

/// A glyph as both sides produced it.
#[derive(Clone, Debug)]
pub(crate) struct GlyphRow {
    /// Emission index.
    pub(crate) index: usize,
    /// Parley's glyph, absent if Parley produced fewer glyphs than Chrome.
    pub(crate) parley: Option<PositionedGlyph>,
    /// Chrome's glyph, absent if Chrome produced fewer glyphs than Parley.
    pub(crate) chrome: Option<PositionedGlyph>,
    /// `parley.x - chrome.x`, when both sides have a glyph here.
    pub(crate) dx: Option<f64>,
    /// `parley.y - chrome.y`, when both sides have a glyph here.
    pub(crate) dy: Option<f64>,
    /// The half-ULP tolerance `dx` is judged against.
    pub(crate) x_tolerance: f32,
    /// The half-ULP tolerance `dy` is judged against.
    pub(crate) y_tolerance: f32,
    /// Whether this glyph agrees on id, style and both axes.
    pub(crate) matches: bool,
    /// Index into [`CaseReport::clusters`] of the Parley cluster owning this glyph.
    pub(crate) cluster: Option<usize>,
}

/// A Parley cluster, and where it sits on the Parley raster.
#[derive(Clone, Debug)]
pub(crate) struct ClusterBox {
    /// Index into [`CaseReport::clusters`], and the key the pastel colour derives from.
    pub(crate) index: usize,
    /// Left edge, in the padded CSS-px space.
    pub(crate) x: f32,
    /// The cluster's advance.
    pub(crate) width: f32,
    /// Top edge of the owning line's box, in the padded CSS-px space.
    pub(crate) top: f32,
    /// Height of the owning line's box.
    pub(crate) height: f32,
    /// The cluster's text.
    pub(crate) text: String,
    /// The glyph ids the cluster shaped to.
    pub(crate) glyph_ids: Vec<u32>,
    /// The cluster's byte range in the case's text.
    pub(crate) text_range: Range<usize>,
    /// Whether any glyph in this cluster disagrees with Chrome beyond tolerance.
    ///
    /// Drives whether the pastel identity tint is shown by default: a matching cluster
    /// is not interesting evidence, so it stays plain until hovered.
    pub(crate) mismatched: bool,
}

/// A single `char` of the case's text, for the hex list.
#[derive(Clone, Debug)]
pub(crate) struct CharEntry {
    /// The character itself.
    pub(crate) ch: char,
    /// The Parley cluster containing it, if any.
    pub(crate) cluster: Option<usize>,
    /// The family the owning run renders in.
    pub(crate) family: &'static str,
}

impl CaseReport {
    /// Lays `golden`'s case out with Parley and analyses the result against Chrome's
    /// recorded output.
    pub(crate) fn build(golden: Golden, font_cx: &mut parley::FontContext) -> Self {
        let mut layout_cx = LayoutContext::new();
        let laid_out = layout(&golden.case, font_cx, &mut layout_cx);
        let parley = parley_output(&laid_out);
        let text: String = golden
            .case
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();

        let mismatch = compare(&parley, &golden.output).err();
        let mut clusters = collect_clusters(&laid_out, &text);
        let glyph_to_cluster = glyph_to_cluster(&clusters, &laid_out);
        let rows = rows(&parley, &golden.output, &glyph_to_cluster);
        for row in &rows {
            if !row.matches {
                if let Some(cluster) = row.cluster.and_then(|index| clusters.get_mut(index)) {
                    cluster.mismatched = true;
                }
            }
        }
        let chars = chars(&text, &golden, &clusters);
        let geometry = geometry(&laid_out, &parley, &golden.output);

        Self {
            golden,
            parley,
            mismatch,
            rows,
            clusters,
            chars,
            geometry,
        }
    }

    /// The coarse bucket this case's failure falls into, or `None` if it passes.
    pub(crate) fn signature(&self) -> Option<FailureSignature> {
        self.mismatch.as_ref().map(FailureSignature::of)
    }

    /// How many glyphs disagree.
    pub(crate) fn divergent(&self) -> usize {
        self.rows.iter().filter(|row| !row.matches).count()
    }

    /// The emission index of the first disagreeing glyph.
    pub(crate) fn first_divergence(&self) -> Option<usize> {
        self.rows
            .iter()
            .find(|row| !row.matches)
            .map(|row| row.index)
    }

    /// The largest `|dx|` over all glyphs, and the largest `|dy|`.
    pub(crate) fn max_delta(&self) -> (f64, f64) {
        self.rows.iter().fold((0.0_f64, 0.0_f64), |(x, y), row| {
            (
                x.max(row.dx.unwrap_or(0.0).abs()),
                y.max(row.dy.unwrap_or(0.0).abs()),
            )
        })
    }
}

/// Walks the layout in the same order [`parley_output`] emits glyphs in, recording each
/// cluster's box.
fn collect_clusters(laid_out: &Layout<()>, text: &str) -> Vec<ClusterBox> {
    let mut boxes = Vec::new();
    for line in laid_out.lines() {
        let metrics = *line.metrics();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            // Accumulated the same way `parley_output` accumulates glyph x, so a box
            // can't drift away from the glyphs it is drawn behind.
            let mut x = f64::from(glyph_run.offset());
            for cluster in glyph_run.run().visual_clusters() {
                let range = cluster.text_range();
                boxes.push(ClusterBox {
                    index: boxes.len(),
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "positions are well within f32 range; f64 is for accumulation only"
                    )]
                    x: x as f32 + PADDING,
                    width: cluster.advance(),
                    top: metrics.block_min_coord + PADDING,
                    height: metrics.block_max_coord - metrics.block_min_coord,
                    text: text.get(range.clone()).unwrap_or_default().to_string(),
                    glyph_ids: cluster.glyphs().map(|glyph| glyph.id).collect(),
                    text_range: range,
                    mismatched: false,
                });
                x += f64::from(cluster.advance());
            }
        }
    }
    boxes
}

/// Maps each emitted glyph index to its owning cluster.
///
/// Both sides of this walk the layout in emission order, so the mapping is positional:
/// cluster *k* owns the next `k.glyphs().count()` glyphs. That holds because v1 has no
/// bidi, which is what makes visual and emission order the same thing.
fn glyph_to_cluster(clusters: &[ClusterBox], laid_out: &Layout<()>) -> Vec<usize> {
    let mut map = Vec::new();
    for cluster in clusters {
        for _ in &cluster.glyph_ids {
            map.push(cluster.index);
        }
    }
    debug_assert_eq!(
        map.len(),
        laid_out
            .lines()
            .flat_map(|line| line.items())
            .filter_map(|item| match item {
                PositionedLayoutItem::GlyphRun(glyph_run) => Some(glyph_run.glyphs().count()),
                PositionedLayoutItem::InlineBox(_) => None,
            })
            .sum::<usize>(),
        "cluster glyph counts must account for every emitted glyph"
    );
    map
}

/// Pairs the two sides' glyphs by emission index, exactly as `compare` does.
fn rows(parley: &GlyphOutput, chrome: &GlyphOutput, glyph_to_cluster: &[usize]) -> Vec<GlyphRow> {
    (0..parley.glyphs.len().max(chrome.glyphs.len()))
        .map(|index| {
            let parley_glyph = parley.glyphs.get(index).copied();
            let chrome_glyph = chrome.glyphs.get(index).copied();
            let (dx, dy, matches) = match (parley_glyph, chrome_glyph) {
                (Some(p), Some(c)) => {
                    let same_style = parley.styles.get(usize::from(p.style))
                        == chrome.styles.get(usize::from(c.style));
                    (
                        Some(f64::from(p.x) - f64::from(c.x)),
                        Some(f64::from(p.y) - f64::from(c.y)),
                        p.id == c.id
                            && same_style
                            && x_matches(f64::from(p.x), c.x)
                            && y_matches(p.y, c.y),
                    )
                }
                _ => (None, None, false),
            };
            GlyphRow {
                index,
                parley: parley_glyph,
                chrome: chrome_glyph,
                dx,
                dy,
                x_tolerance: chrome_glyph.map_or(0.0, |g| half_ulp_6sig(g.x)),
                y_tolerance: chrome_glyph.map_or(0.0, |g| half_ulp_6sig(g.y)),
                matches,
                cluster: glyph_to_cluster.get(index).copied(),
            }
        })
        .collect()
}

/// Builds the hex list's entries, attributing each `char` to its cluster and its run.
fn chars(text: &str, golden: &Golden, clusters: &[ClusterBox]) -> Vec<CharEntry> {
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for cluster in clusters {
        for offset in cluster.text_range.clone() {
            owner.insert(offset, cluster.index);
        }
    }

    // Runs are laid out as one concatenated string, so a char's run is found by walking
    // the run lengths. v1's grammar carries no per-run family, but resolving it per run
    // is what multi-font support needs, so the lookup is written that way already.
    let mut run_ends = Vec::with_capacity(golden.case.runs.len());
    let mut end = 0;
    for run in &golden.case.runs {
        end += run.text.len();
        run_ends.push(end);
    }

    text.char_indices()
        .map(|(offset, ch)| {
            let run = run_ends.iter().position(|&end| offset < end).unwrap_or(0);
            CharEntry {
                ch,
                cluster: owner.get(&offset).copied(),
                family: family_of_run(run),
            }
        })
        .collect()
}

/// The family a run renders in.
///
/// v1's `Run` has no family of its own — there is exactly one registered font — but
/// every caller asks per run, so adding one is a change here rather than everywhere.
fn family_of_run(_run: usize) -> &'static str {
    FONTS[0].family
}

/// The canvas both sides are rasterised into.
///
/// Sized to cover *both* sides: a case whose Chrome glyphs run past Parley's must not
/// have its evidence cropped away.
fn geometry(laid_out: &Layout<()>, parley: &GlyphOutput, chrome: &GlyphOutput) -> Geometry {
    let max_size = parley
        .styles
        .iter()
        .chain(&chrome.styles)
        .fold(0.0_f32, |acc, style| acc.max(style.font_size));
    let (max_x, max_y) = parley
        .glyphs
        .iter()
        .chain(&chrome.glyphs)
        .fold((0.0_f32, 0.0_f32), |(x, y), glyph| {
            (x.max(glyph.x), y.max(glyph.y))
        });
    Geometry {
        width: (laid_out.width().max(max_x + max_size) + PADDING * 2.0).ceil(),
        height: (laid_out.height().max(max_y + max_size * 0.5) + PADDING * 2.0).ceil(),
    }
}
