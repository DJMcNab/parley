// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The index page.
//!
//! Unlike a case page, this is *not* self-contained: it references each case's two
//! raster PNGs from disk and composites the overlay thumbnail in CSS, exactly as the
//! case page does. Baking a separate thumbnail would mean a second implementation of
//! the overlay, which is a worse problem than a large index.
//!
//! Sections are the input's source directory, since the useful grouping — one bucket per
//! distinct bug — depends on a classification the failure minimiser doesn't emit yet.
//! Ordering does the work in the meantime: ad-hoc inputs (a fresh fuzz haul) first, then
//! the tracked known failures, then the directories expected to pass, with anything that
//! contradicts its directory hoisted to the top of the report.

use std::fmt::Write as _;

use crate::html::{TINT_FILTERS, escape};

/// One case's row.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// The case's name, e.g. `seed_0007`.
    pub(crate) name: String,
    /// The source directory this case came from, or `adhoc`.
    pub(crate) section: String,
    /// Path to the case page, relative to the index.
    pub(crate) href: String,
    /// Path to Chrome's raster, relative to the index.
    pub(crate) chrome_png: String,
    /// Path to Parley's raster, relative to the index.
    pub(crate) parley_png: String,
    /// The raster's CSS-px size, for the thumbnail's aspect ratio.
    pub(crate) width: f32,
    /// See [`Self::width`].
    pub(crate) height: f32,
    /// Whether the case's directory expects it to fail.
    pub(crate) expected_failure: bool,
    /// Whether it actually failed.
    pub(crate) failed: bool,
    /// Parley's glyph count.
    pub(crate) parley_glyphs: usize,
    /// Chrome's glyph count.
    pub(crate) chrome_glyphs: usize,
    /// How many glyphs disagree.
    pub(crate) divergent: usize,
    /// The largest `|dx|`.
    pub(crate) max_dx: f64,
    /// The largest `|dy|`.
    pub(crate) max_dy: f64,
    /// The emission index of the first disagreeing glyph.
    pub(crate) first_divergence: Option<usize>,
    /// The golden's note, if it has one.
    pub(crate) note: Option<String>,
    /// The failure's coarse signature, if it failed.
    pub(crate) signature: Option<String>,
}

impl Entry {
    /// Whether the case's actual result contradicts what its directory expects.
    pub(crate) fn is_surprise(&self) -> bool {
        self.failed != self.expected_failure
    }
}

/// A case that could not be analysed at all.
#[derive(Clone, Debug)]
pub(crate) struct Failed {
    /// The input path.
    pub(crate) path: String,
    /// Why it could not be used.
    pub(crate) error: String,
}

/// The width, in CSS px, an index thumbnail is drawn at.
const THUMBNAIL_WIDTH: f32 = 460.0;

/// Renders the index.
pub(crate) fn render(entries: &[Entry], failed: &[Failed]) -> String {
    let mut out = String::new();
    writeln!(out, "<!doctype html>").unwrap();
    writeln!(out, "<html lang=\"en\">").unwrap();
    writeln!(out, "<meta charset=\"utf-8\">").unwrap();
    writeln!(out, "<title>Glyph positioning report</title>").unwrap();
    writeln!(out, "<style>{}</style>", include_str!("index.css")).unwrap();
    writeln!(out, "{TINT_FILTERS}").unwrap();
    writeln!(out, "<h1>Glyph positioning report</h1>").unwrap();

    let failures = entries.iter().filter(|entry| entry.failed).count();
    let surprises = entries.iter().filter(|entry| entry.is_surprise()).count();
    writeln!(
        out,
        "<p class=\"summary\">{} case(s): {} diverge from Chrome, {} match. \
         {surprises} contradict their directory.</p>",
        entries.len(),
        failures,
        entries.len() - failures,
    )
    .unwrap();

    if !failed.is_empty() {
        writeln!(out, "<h2>Could not be analysed</h2><ul class=\"errors\">").unwrap();
        for entry in failed {
            writeln!(
                out,
                "<li><code>{}</code> — {}</li>",
                escape(&entry.path),
                escape(&entry.error)
            )
            .unwrap();
        }
        writeln!(out, "</ul>").unwrap();
    }

    for section in sections(entries) {
        let mut rows: Vec<&Entry> = entries
            .iter()
            .filter(|entry| entry.section == section)
            .collect();
        // Surprises first, then ordinary failures, then passes; alphabetical within
        // each, so a rerun's diff is readable.
        rows.sort_by(|a, b| {
            b.is_surprise()
                .cmp(&a.is_surprise())
                .then_with(|| b.failed.cmp(&a.failed))
                .then_with(|| a.name.cmp(&b.name))
        });

        writeln!(out, "<h2>{}</h2>", escape(&section)).unwrap();
        for entry in rows {
            card(&mut out, entry);
        }
    }

    out
}

/// The sections present, in report order.
fn sections(entries: &[Entry]) -> Vec<String> {
    const ORDER: &[&str] = &[
        "adhoc",
        "known_failing",
        "handwritten",
        "regressions",
        "generated",
    ];
    let mut present: Vec<String> = entries
        .iter()
        .map(|entry| entry.section.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    present.sort_by_key(|section| {
        ORDER
            .iter()
            .position(|known| known == section)
            .unwrap_or(ORDER.len())
    });
    present
}

/// One case's card: thumbnail plus the numbers worth scanning.
fn card(out: &mut String, entry: &Entry) {
    let height = entry.height * (THUMBNAIL_WIDTH / entry.width);
    let size = format!("width:{THUMBNAIL_WIDTH}px;height:{height}px");
    let status = match (entry.failed, entry.expected_failure) {
        (true, true) => ("known-fail", "diverges (expected)"),
        (true, false) => ("surprise", "DIVERGES — expected to match"),
        (false, true) => ("surprise", "MATCHES — expected to diverge"),
        (false, false) => ("pass", "matches"),
    };

    writeln!(out, "<div class=\"card {}\">", status.0).unwrap();
    writeln!(
        out,
        "<a class=\"thumb overlay\" href=\"{}\" style=\"{size}\">\
         <img class=\"tint chrome\" alt=\"\" src=\"{}\" style=\"{size}\">\
         <img class=\"tint parley\" alt=\"\" src=\"{}\" style=\"{size}\"></a>",
        escape(&entry.href),
        escape(&entry.chrome_png),
        escape(&entry.parley_png),
    )
    .unwrap();

    writeln!(out, "<div class=\"detail\">").unwrap();
    writeln!(
        out,
        "<h3><a href=\"{}\">{}</a></h3>",
        escape(&entry.href),
        escape(&entry.name)
    )
    .unwrap();
    writeln!(out, "<p class=\"status\">{}</p>", escape(status.1)).unwrap();
    writeln!(
        out,
        "<dl><dt>glyphs</dt><dd>Parley {}, Chrome {}</dd>\
         <dt>divergent</dt><dd>{}</dd>\
         <dt>max |Δ|</dt><dd>x {:.6}px, y {:.6}px</dd>\
         <dt>first</dt><dd>{}</dd>\
         <dt>signature</dt><dd>{}</dd></dl>",
        entry.parley_glyphs,
        entry.chrome_glyphs,
        entry.divergent,
        entry.max_dx,
        entry.max_dy,
        entry
            .first_divergence
            .map_or_else(|| "—".to_string(), |index| format!("glyph {index}")),
        escape(entry.signature.as_deref().unwrap_or("—")),
    )
    .unwrap();
    if let Some(note) = &entry.note {
        writeln!(out, "<p class=\"note\">{}</p>", escape(note)).unwrap();
    }
    writeln!(out, "</div></div>").unwrap();
}
