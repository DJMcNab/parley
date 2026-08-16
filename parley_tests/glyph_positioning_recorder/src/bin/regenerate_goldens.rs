// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Re-records every golden's Chrome output in place.
//!
//! **Recording only.** This never compares against Parley: Chrome is the oracle, and
//! `cargo test` is the place a verdict is rendered. Re-running must produce
//! byte-identical files — that idempotency check is the real test of Chrome's
//! determinism.
//!
//! ```sh
//! container/run.sh regenerate_goldens [filter]
//! ```
//!
//! or, against an already-running container (see `container/run.sh`):
//!
//! ```sh
//! cargo run -p parley_glyph_positioning_recorder --bin regenerate_goldens -- [filter]
//! ```
//!
//! See "`src/bin/regenerate_goldens.rs`" in
//! `doc/glyph-positioning-chrome-parity-phase4.md` and
//! `doc/glyph-positioning-recorder-agent.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parley_glyph_positioning_cases::{Case, Golden};
use parley_glyph_positioning_recorder::Result;
use parley_glyph_positioning_recorder::driver::{Config, Recorder};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let filter = std::env::args().nth(1);
    let root = goldens_dir();

    let mut work = collect_existing(&root)?;

    if let Some(filter) = &filter {
        work.retain(|path, _| path.to_string_lossy().contains(filter.as_str()));
        if work.is_empty() {
            return Err(format!(
                "filter {filter:?} matched no goldens under {}",
                root.display()
            )
            .into());
        }
    }

    println!("recording {} golden(s)", work.len());
    let mut recorder = Recorder::attach(Config::from_env()).await?;
    let result = record_all(&mut recorder, &work).await;
    let _ = recorder.close().await;
    result
}

/// Captures and writes every entry in `work`.
async fn record_all(
    recorder: &mut Recorder,
    work: &BTreeMap<PathBuf, (Case, Option<String>)>,
) -> Result<()> {
    for (path, (case, note)) in work {
        let recording = recorder
            .capture(case)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let golden = Golden {
            case: case.clone(),
            note: note.clone(),
            output: recording.output,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, golden.write())?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// The checked-in golden corpus, whatever it currently holds.
fn goldens_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/glyph_positioning")
}

/// Reads the `Case` and `note` out of every existing golden, in every directory.
///
/// One uniform rule across `handwritten/`, `regressions/`, `generated/` and
/// `known_failing/`: whatever is on disk has its case re-recorded in place. A
/// handwritten case is therefore authored by hand with `styles 0` / `fragments 0` and
/// filled in by the first run, and a promoted fuzz regression already arrives in
/// exactly that shape.
///
/// This tool does **not** invent new `generated/` seeds on its own. It never compares
/// against Parley — that is `cargo test`'s job — so a blindly-added seed would go into
/// the "expected to pass" corpus without anything having checked that it does. Most
/// seeds do pass, but the ones that don't are exactly the cases worth classifying by
/// hand into `known_failing/` with a note. Growing `generated/` therefore needs an
/// explicit pass/fail search against a live container.
fn collect_existing(root: &Path) -> Result<BTreeMap<PathBuf, (Case, Option<String>)>> {
    let mut work = BTreeMap::new();
    for path in text_files(root)? {
        let text = std::fs::read_to_string(&path)?;
        let golden =
            Golden::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
        work.insert(path, (golden.case, golden.note));
    }
    Ok(work)
}

/// Every `*.txt` under `dir`, recursively. A missing `dir` is not an error: the corpus
/// starts empty.
fn text_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(files);
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(text_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "txt") {
            files.push(path);
        }
    }
    Ok(files)
}
