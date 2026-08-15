// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Generates a browsable HTML report for glyph-positioning Chrome-parity cases.
//!
//! ```text
//! cargo run -p parley_glyph_positioning_report            # the checked-in corpus
//! cargo run -p parley_glyph_positioning_report -- PATH... # specific goldens or dirs
//! ```
//!
//! Each case gets a page showing Chrome's recorded glyphs, Parley's current glyphs, the
//! two overlaid, and the recorder's DOM live in your browser — plus the case's
//! characters grouped by Parley cluster and a per-glyph diff table. An `index.html`
//! ties them together; the case pages deliberately do not link back to it, so one page
//! can be shared on its own.
//!
//! Paths may be golden files or directories (walked recursively), so a fuzz haul's
//! `seed_*/case.txt` artifacts work as-is. Parley is re-run here, so the verdict is what
//! Parley does *now*, not what the case's directory claims.

mod analysis;
mod html;
mod index;
mod page;
mod raster;

use std::path::{Path, PathBuf};

use analysis::CaseReport;
use parley_glyph_positioning_cases::Golden;
use parley_glyph_positioning_extract::font_context;

/// Where a case came from, which is what the index sections on.
const CORPUS_DIRS: &[&str] = &["handwritten", "regressions", "generated", "known_failing"];

fn main() -> std::process::ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}\n\nusage: [PATH...] [--out DIR]");
            return std::process::ExitCode::FAILURE;
        }
    };

    match run(&args) {
        Ok(summary) => {
            println!("{summary}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The command line.
#[derive(Debug)]
struct Args {
    /// Golden files and directories to report on. Empty means the checked-in corpus.
    inputs: Vec<PathBuf>,
    /// Where the report is written.
    out: PathBuf,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut inputs = Vec::new();
        let mut out = None;
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => {
                    out = Some(PathBuf::from(
                        args.next().ok_or("--out needs a directory")?,
                    ));
                }
                other if other.starts_with("--") => {
                    return Err(format!("unknown option {other}"));
                }
                other => inputs.push(PathBuf::from(other)),
            }
        }
        Ok(Self {
            inputs,
            out: out.unwrap_or_else(default_out_dir),
        })
    }
}

/// `target/glyph_positioning_report/`, which is gitignored by virtue of being under
/// `target/`.
fn default_out_dir() -> PathBuf {
    workspace_root().join("target/glyph_positioning_report")
}

/// The repository root, derived from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("this crate sits two directories below the workspace root")
        .to_path_buf()
}

/// `parley_tests/tests/glyph_positioning/`, the checked-in golden corpus.
fn corpus_root() -> PathBuf {
    workspace_root().join("parley_tests/tests/glyph_positioning")
}

/// An input golden, and where it came from.
#[derive(Debug)]
struct Input {
    path: PathBuf,
    /// The index section: a corpus directory name, or `adhoc`.
    section: String,
    /// The case's display name, and the stem of its output files.
    name: String,
}

fn run(args: &Args) -> Result<String, String> {
    let inputs = collect_inputs(args)?;
    if inputs.is_empty() {
        return Err("no golden files found".to_string());
    }

    std::fs::create_dir_all(&args.out).map_err(|error| format!("{}: {error}", args.out.display()))?;

    let mut font_cx = font_context();
    let fonts = raster::Fonts::new();
    let mut entries = Vec::new();
    let mut failed = Vec::new();

    for input in &inputs {
        match generate(input, args, &mut font_cx, &fonts) {
            Ok(entry) => entries.push(entry),
            Err(error) => failed.push(index::Failed {
                path: input.path.display().to_string(),
                error,
            }),
        }
    }

    let index_html = index::render(&entries, &failed);
    let index_path = args.out.join("index.html");
    std::fs::write(&index_path, index_html)
        .map_err(|error| format!("{}: {error}", index_path.display()))?;

    let diverging = entries.iter().filter(|entry| entry.failed).count();
    let surprises = entries.iter().filter(|entry| entry.is_surprise()).count();
    Ok(format!(
        "{} case(s): {diverging} diverge, {} match, {surprises} contradict their directory\
         {}\n{}",
        entries.len(),
        entries.len() - diverging,
        if failed.is_empty() {
            String::new()
        } else {
            format!(", {} could not be analysed", failed.len())
        },
        index_path.display(),
    ))
}

/// Analyses one case and writes its page and rasters.
fn generate(
    input: &Input,
    args: &Args,
    font_cx: &mut parley::FontContext,
    fonts: &raster::Fonts,
) -> Result<index::Entry, String> {
    let text = std::fs::read_to_string(&input.path).map_err(|error| error.to_string())?;
    let golden = Golden::parse(&text).map_err(|error| error.to_string())?;
    let note = golden.note.clone();
    let report = CaseReport::build(golden, font_cx);

    let section_dir = args.out.join(&input.section);
    std::fs::create_dir_all(&section_dir).map_err(|error| error.to_string())?;

    let chrome_png = raster::render(&report.golden.output, report.geometry, fonts);
    let parley_png = raster::render(&report.parley, report.geometry, fonts);
    // The rasters are inlined into the page (so a page can be shared on its own) *and*
    // written out (so the index can composite thumbnails without pulling in a megabyte
    // of base64 per card). Same bytes, one generator, two sinks.
    let chrome_name = format!("{}.chrome.png", input.name);
    let parley_name = format!("{}.parley.png", input.name);
    std::fs::write(section_dir.join(&chrome_name), &chrome_png)
        .map_err(|error| error.to_string())?;
    std::fs::write(section_dir.join(&parley_name), &parley_png)
        .map_err(|error| error.to_string())?;

    let page_name = format!("{}.html", input.name);
    let page_html = page::render(&report, &input.name, &chrome_png, &parley_png);
    std::fs::write(section_dir.join(&page_name), page_html).map_err(|error| error.to_string())?;

    let (max_dx, max_dy) = report.max_delta();
    Ok(index::Entry {
        name: input.name.clone(),
        section: input.section.clone(),
        href: format!("{}/{page_name}", input.section),
        chrome_png: format!("{}/{chrome_name}", input.section),
        parley_png: format!("{}/{parley_name}", input.section),
        width: report.geometry.width,
        height: report.geometry.height,
        expected_failure: input.section == "known_failing",
        failed: report.mismatch.is_some(),
        parley_glyphs: report.parley.glyphs.len(),
        chrome_glyphs: report.golden.output.glyphs.len(),
        divergent: report.divergent(),
        max_dx,
        max_dy,
        first_divergence: report.first_divergence(),
        note,
        signature: report.signature().map(|signature| signature.to_string()),
    })
}

/// Expands the command line into the cases to report on.
fn collect_inputs(args: &Args) -> Result<Vec<Input>, String> {
    let mut inputs = Vec::new();
    if args.inputs.is_empty() {
        let root = corpus_root();
        for dir in CORPUS_DIRS {
            let path = root.join(dir);
            if path.is_dir() {
                collect_from(&path, dir, &mut inputs)?;
            }
        }
    } else {
        for path in &args.inputs {
            let section = section_for(path);
            if path.is_dir() {
                collect_from(path, &section, &mut inputs)?;
            } else {
                let name = case_name(path, &inputs);
                inputs.push(Input {
                    name,
                    path: path.clone(),
                    section,
                });
            }
        }
    }
    Ok(inputs)
}

/// Walks `dir` recursively, taking every file as a candidate golden.
///
/// Extension is not filtered on: fuzz artifacts are `case.txt`, but nothing guarantees
/// that stays true, and a file that turns out not to be a golden becomes a visible
/// error row rather than a silent omission.
fn collect_from(dir: &Path, section: &str, inputs: &mut Vec<Input>) -> Result<(), String> {
    let mut paths = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        paths.push(entry.path());
    }
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_from(&path, section, inputs)?;
        } else {
            let name = case_name(&path, inputs);
            inputs.push(Input {
                name,
                path,
                section: section.to_string(),
            });
        }
    }
    Ok(())
}

/// The section an explicitly-named path belongs to: its corpus directory if it is one
/// of those, otherwise `adhoc`.
fn section_for(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let corpus = corpus_root()
        .canonicalize()
        .unwrap_or_else(|_| corpus_root());
    if let Ok(relative) = canonical.strip_prefix(&corpus)
        && let Some(first) = relative.iter().next()
        && let Some(name) = first.to_str()
        && CORPUS_DIRS.contains(&name)
    {
        return name.to_string();
    }
    "adhoc".to_string()
}

/// A case's name, unique within the report.
///
/// Fuzz artifacts are all called `case.txt` under a `seed_<n>/` directory, so a bare
/// file stem would collide across every one of them; the parent directory's name is
/// what actually identifies those. A counter suffix breaks any remaining tie rather
/// than letting one case silently overwrite another.
fn case_name(path: &Path, existing: &[Input]) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("case");
    let base = if stem == "case" {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(stem)
            .to_string()
    } else {
        stem.to_string()
    };

    let mut name = base.clone();
    let mut counter = 2;
    while existing.iter().any(|input| input.name == name) {
        name = format!("{base}-{counter}");
        counter += 1;
    }
    name
}
