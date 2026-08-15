// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The per-case page.
//!
//! A page is self-contained apart from its fonts: both rasters are inlined as `data:`
//! URIs, and there is deliberately no link back to the index, so a single file can be
//! handed to someone as the whole story of one case.
//!
//! Font bytes are the exception — they are fetched from the face's canonical URL, since
//! the corpus is expected to outgrow what can reasonably be vendored, let alone inlined
//! per page. `@font-face` cannot carry a subresource-integrity hash, so the page fetches
//! the bytes itself, checks their SHA-256 against the digest baked in here, and only
//! then constructs the `FontFace`. A face that fails to arrive, or arrives wrong,
//! replaces the live-DOM panel with an error rather than silently rendering in a
//! fallback face — which would look like a Parley bug.

use std::fmt::Write as _;

use icu_properties::CodePointMapData;
use icu_properties::props::GeneralCategory;
use parley_glyph_positioning_cases::{FONTS, SupportedFont, container_css, run_css};

use crate::analysis::{CaseReport, PADDING};
use crate::html::{TINT_FILTERS, base64, cluster_hue, escape};

/// The page's stylesheet.
const STYLE: &str = include_str!("page.css");
/// The page's script: font pinning, and the hover linkage.
const SCRIPT: &str = include_str!("page.js");

/// Renders the page for one case.
pub(crate) fn render(
    report: &CaseReport,
    name: &str,
    chrome_png: &[u8],
    parley_png: &[u8],
) -> String {
    let mut out = String::new();
    let geometry = report.geometry;

    writeln!(out, "<!doctype html>").unwrap();
    writeln!(out, "<html lang=\"en\">").unwrap();
    writeln!(out, "<head>").unwrap();
    writeln!(out, "<meta charset=\"utf-8\">").unwrap();
    writeln!(out, "<title>{} — glyph positioning</title>", escape(name)).unwrap();
    writeln!(out, "<style>{STYLE}</style>").unwrap();
    writeln!(out, "</head>").unwrap();
    writeln!(out, "<body>").unwrap();
    writeln!(out, "{TINT_FILTERS}").unwrap();

    header(&mut out, report, name);
    panels(&mut out, report, chrome_png, parley_png);
    hex_list(&mut out, report);
    diff_table(&mut out, report);
    raw_golden(&mut out, report);

    writeln!(
        out,
        "<script>\nconst FACES = {};\nconst GEOMETRY = {{width: {}, height: {}}};\n{SCRIPT}\n</script>",
        faces_json(report),
        geometry.width,
        geometry.height,
    )
    .unwrap();
    writeln!(out, "</body>\n</html>").unwrap();
    out
}

/// The case's identity, verdict and parameters.
fn header(out: &mut String, report: &CaseReport, name: &str) {
    let case = &report.golden.case;
    let (verdict_class, verdict) = match &report.mismatch {
        None => ("pass", "matches Chrome".to_string()),
        Some(mismatch) => (
            "fail",
            format!(
                "diverges from Chrome — {}",
                report
                    .signature()
                    .map_or_else(String::new, |signature| signature.to_string())
            )
            .replace("&", "&amp;")
                + &format!(
                    " ({} of {} glyphs)",
                    report.divergent(),
                    report.rows.len().max(mismatch_total(mismatch))
                ),
        ),
    };

    writeln!(out, "<h1>{}</h1>", escape(name)).unwrap();
    writeln!(
        out,
        "<p class=\"verdict {verdict_class}\">{}</p>",
        escape(&verdict)
    )
    .unwrap();

    writeln!(out, "<dl class=\"meta\">").unwrap();
    writeln!(out, "<dt>seed</dt><dd>{}</dd>", case.seed).unwrap();
    writeln!(out, "<dt>width</dt><dd>{}px</dd>", case.width).unwrap();
    writeln!(
        out,
        "<dt>glyphs</dt><dd>Parley {}, Chrome {}</dd>",
        report.parley.glyphs.len(),
        report.golden.output.glyphs.len()
    )
    .unwrap();
    let (max_dx, max_dy) = report.max_delta();
    writeln!(
        out,
        "<dt>max |Δ|</dt><dd>x {max_dx:.6}px, y {max_dy:.6}px</dd>"
    )
    .unwrap();
    for (index, run) in case.runs.iter().enumerate() {
        writeln!(
            out,
            "<dt>run {index}</dt><dd>{}px, letter-spacing {}px, word-spacing {}px, {} chars</dd>",
            run.font_size,
            run.letter_spacing,
            run.word_spacing,
            run.text.chars().count()
        )
        .unwrap();
    }
    if let Some(note) = &report.golden.note {
        writeln!(out, "<dt>note</dt><dd>{}</dd>", escape(note)).unwrap();
    }
    writeln!(out, "</dl>").unwrap();
}

/// How many glyphs a mismatch was over, for the verdict line.
fn mismatch_total(mismatch: &parley_glyph_positioning_cases::Mismatch) -> usize {
    match mismatch {
        parley_glyph_positioning_cases::Mismatch::GlyphCount { parley, chrome } => {
            (*parley).max(*chrome)
        }
        parley_glyph_positioning_cases::Mismatch::Glyphs { total, .. } => *total,
    }
}

/// The four renderings, as a 2×2 grid: the two sides on the top row, and below them
/// what they add up to (the overlay) and what the reader's own browser makes of the
/// same DOM.
///
/// The cells are the rasters' natural CSS-px size — they cannot be scaled to fit,
/// since the cluster boxes are positioned in layout coordinates — so a wide case falls
/// back to a single column rather than being squeezed.
fn panels(out: &mut String, report: &CaseReport, chrome_png: &[u8], parley_png: &[u8]) {
    let geometry = report.geometry;
    let chrome_uri = format!("data:image/png;base64,{}", base64(chrome_png));
    let parley_uri = format!("data:image/png;base64,{}", base64(parley_png));
    let size = format!("width:{}px;height:{}px", geometry.width, geometry.height);

    writeln!(
        out,
        "<div id=\"font-error\" class=\"font-error\" hidden></div>"
    )
    .unwrap();
    // Two cells plus the column gap: what caps the flex container at two per row.
    writeln!(
        out,
        "<div class=\"panels\" style=\"max-width:{}px\">",
        geometry.width * 2.0 + 24.0
    )
    .unwrap();

    writeln!(
        out,
        "<figure class=\"cell\" style=\"width:{}px\"><h2>Chrome (recorded)</h2>",
        geometry.width
    )
    .unwrap();
    writeln!(
        out,
        "<div class=\"panel\" style=\"{size}\"><img alt=\"Chrome's glyphs\" src=\"{chrome_uri}\" \
         style=\"{size}\"></div>"
    )
    .unwrap();
    writeln!(
        out,
        "<figcaption>The golden's recorded glyphs — the reference every other panel is \
         judged against.</figcaption></figure>"
    )
    .unwrap();

    writeln!(
        out,
        "<figure class=\"cell\" style=\"width:{}px\"><h2>Parley (now)</h2>",
        geometry.width
    )
    .unwrap();
    writeln!(out, "<div class=\"panel\" style=\"{size}\">").unwrap();
    for cluster in &report.clusters {
        writeln!(
            out,
            "<div class=\"cbox{}\" data-cluster=\"{}\" style=\"left:{}px;top:{}px;width:{}px;\
             height:{}px;--hue:{}\" title=\"cluster {} — {}\"></div>",
            if cluster.mismatched { " mismatch" } else { "" },
            cluster.index,
            cluster.x,
            cluster.top,
            cluster.width,
            cluster.height,
            cluster_hue(cluster.index),
            cluster.index,
            escape(&cluster.text)
        )
        .unwrap();
    }
    writeln!(
        out,
        "<img alt=\"Parley's glyphs\" src=\"{parley_uri}\" style=\"{size}\">"
    )
    .unwrap();
    writeln!(out, "</div>").unwrap();
    writeln!(
        out,
        "<figcaption>What Parley produces today. A cluster that diverges from Chrome gets a \
         pastel box behind its glyphs; hover any cluster to light it up everywhere.\
         </figcaption></figure>"
    )
    .unwrap();

    writeln!(
        out,
        "<figure class=\"cell\" style=\"width:{}px\"><h2>Overlay</h2>",
        geometry.width
    )
    .unwrap();
    writeln!(
        out,
        "<div class=\"panel overlay\" style=\"{size}\">\
         <img class=\"tint chrome\" alt=\"Chrome's glyphs, tinted red\" src=\"{chrome_uri}\" \
         style=\"{size}\">\
         <img class=\"tint parley\" alt=\"Parley's glyphs, tinted blue\" src=\"{parley_uri}\" \
         style=\"{size}\"></div>"
    )
    .unwrap();
    writeln!(
        out,
        "<figcaption>Chrome in red, Parley in blue, multiplied: agreement reads as near-black, \
         disagreement as colour fringing. Zoom in — most differences here are sub-pixel.\
         </figcaption></figure>"
    )
    .unwrap();

    writeln!(
        out,
        "<figure class=\"cell\" style=\"width:{}px\"><h2>Live DOM (your browser)</h2>",
        geometry.width
    )
    .unwrap();
    writeln!(
        out,
        "<div class=\"panel live\" style=\"{size};padding:{PADDING}px\">"
    )
    .unwrap();
    writeln!(
        out,
        "<div class=\"live-content\" style=\"{}\">",
        escape(&container_css(&report.golden.case))
    )
    .unwrap();
    for run in &report.golden.case.runs {
        write!(
            out,
            "<span style=\"{}\">{}</span>",
            escape(&run_css(run)),
            escape(&run.text)
        )
        .unwrap();
    }
    writeln!(out, "</div></div>").unwrap();
    writeln!(
        out,
        "<figcaption>The recorder's DOM and CSS, rendered by whatever browser you are reading \
         this in — <strong>not</strong> the pinned Chrome the golden was recorded from, which \
         runs with <code>--font-render-hinting=none</code>. Pixel differences here are not \
         evidence of anything.</figcaption></figure>"
    )
    .unwrap();

    writeln!(out, "</div>").unwrap();
}

/// The per-character hex list, pastel-grouped by owning cluster.
fn hex_list(out: &mut String, report: &CaseReport) {
    writeln!(out, "<h2>Characters</h2>").unwrap();
    writeln!(
        out,
        "<p class=\"caption\">One entry per <code>char</code>, in logical order. A tint means \
         the owning Parley cluster diverges from Chrome; hover any entry to light up that \
         cluster everywhere.</p>"
    )
    .unwrap();
    writeln!(out, "<div class=\"chars\">").unwrap();
    for entry in &report.chars {
        let cluster = entry.cluster;
        let hue = cluster.map_or(0.0, cluster_hue);
        let mismatched = cluster
            .and_then(|index| report.clusters.get(index))
            .is_some_and(|cluster| cluster.mismatched);
        let attrs = match cluster {
            None => " class=\"ch orphan\"".to_string(),
            Some(index) => format!(
                " class=\"ch{}\" data-cluster=\"{index}\"",
                if mismatched { " mismatch" } else { "" }
            ),
        };
        writeln!(
            out,
            "<span{attrs} style=\"--hue:{hue}\"><code>U+{:04X}</code>\
             <span class=\"sample\" style=\"font-family:'{}'\">{}</span></span>",
            u32::from(entry.ch),
            escape(entry.family),
            escape(&displayable(entry.ch))
        )
        .unwrap();
    }
    writeln!(out, "</div>").unwrap();
}

/// How a `char` is shown in the hex list.
///
/// A lone combining mark gets a dotted-circle base (U+25CC), the convention for showing
/// a mark in isolation — without one it drifts onto whatever precedes it in the DOM,
/// which in the failing cases is exactly the character you are trying to read. A space
/// gets an open box, so an empty cell can't be mistaken for a missing entry.
fn displayable(ch: char) -> String {
    let category = CodePointMapData::<GeneralCategory>::new().get(ch);
    match category {
        GeneralCategory::NonspacingMark
        | GeneralCategory::SpacingMark
        | GeneralCategory::EnclosingMark => format!("\u{25cc}{ch}"),
        GeneralCategory::SpaceSeparator => "\u{2423}".to_string(),
        _ => ch.to_string(),
    }
}

/// The per-glyph diff table.
fn diff_table(out: &mut String, report: &CaseReport) {
    writeln!(out, "<h2>Glyphs</h2>").unwrap();
    writeln!(out, "<div class=\"scroll\">").unwrap();
    writeln!(out, "<table>").unwrap();
    writeln!(
        out,
        "<thead><tr><th>#</th><th>cluster</th><th>id</th><th>x (Chrome)</th><th>x (Parley)</th>\
         <th>dx</th><th>±x</th><th>y (Chrome)</th><th>y (Parley)</th><th>dy</th><th>±y</th></tr>\
         </thead><tbody>"
    )
    .unwrap();
    for row in &report.rows {
        let cluster = row.cluster.and_then(|index| report.clusters.get(index));
        let attrs = cluster.map_or_else(
            || format!("class=\"{}\"", if row.matches { "" } else { "bad" }),
            |cluster| {
                format!(
                    "class=\"{}\" data-cluster=\"{}\" style=\"--hue:{}\"",
                    if row.matches { "" } else { "bad" },
                    cluster.index,
                    cluster_hue(cluster.index)
                )
            },
        );
        let id = match (row.parley, row.chrome) {
            (Some(p), Some(c)) if p.id == c.id => p.id.to_string(),
            (p, c) => format!(
                "{} / {}",
                c.map_or_else(|| "—".to_string(), |g| g.id.to_string()),
                p.map_or_else(|| "—".to_string(), |g| g.id.to_string())
            ),
        };
        let cluster_class = if cluster.is_some_and(|cluster| cluster.mismatched) {
            "cluster mismatch"
        } else {
            "cluster"
        };
        writeln!(
            out,
            "<tr {attrs}><td>{}</td><td class=\"{cluster_class}\">{}</td><td>{id}</td>\
             <td>{}</td><td>{}</td><td class=\"delta\">{}</td><td>{}</td>\
             <td>{}</td><td>{}</td><td class=\"delta\">{}</td><td>{}</td></tr>",
            row.index,
            cluster.map_or_else(
                || "—".to_string(),
                |cluster| format!("#{} {}", cluster.index, escape(&cluster.text))
            ),
            row.chrome
                .map_or_else(|| "—".to_string(), |g| g.x.to_string()),
            row.parley
                .map_or_else(|| "—".to_string(), |g| g.x.to_string()),
            row.dx
                .map_or_else(|| "—".to_string(), |dx| format!("{dx:+.6}")),
            row.x_tolerance,
            row.chrome
                .map_or_else(|| "—".to_string(), |g| g.y.to_string()),
            row.parley
                .map_or_else(|| "—".to_string(), |g| g.y.to_string()),
            row.dy
                .map_or_else(|| "—".to_string(), |dy| format!("{dy:+.6}")),
            row.y_tolerance,
        )
        .unwrap();
    }
    writeln!(out, "</tbody></table></div>").unwrap();
}

/// The golden file's own text, for pasting into an issue or a `known_failing/` entry.
fn raw_golden(out: &mut String, report: &CaseReport) {
    writeln!(
        out,
        "<details><summary>Golden file</summary><pre>{}</pre></details>",
        escape(&report.golden.write())
    )
    .unwrap();
}

/// The faces this case's runs need, as the JSON the page's font loader consumes.
fn faces_json(report: &CaseReport) -> String {
    let mut entries = Vec::new();
    for font in used_faces(report) {
        entries.push(format!(
            "{{\"family\": \"{}\", \"url\": \"{}\", \"sha256\": \"{}\"}}",
            font.family, font.url, font.sha256
        ));
    }
    format!("[{}]", entries.join(", "))
}

/// Which registered faces this case actually uses.
///
/// A page carries only its own case's faces, so page weight tracks case complexity
/// rather than corpus size. v1's grammar has one family, but the shape generalises.
fn used_faces(report: &CaseReport) -> Vec<&'static SupportedFont> {
    let families: Vec<&str> = report
        .chars
        .iter()
        .map(|entry| entry.family)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    FONTS
        .iter()
        .filter(|font| families.contains(&font.family))
        .collect()
}
