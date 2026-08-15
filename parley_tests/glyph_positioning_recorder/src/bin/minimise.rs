// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Signature-preserving minimisation of fuzz-loop failure artifacts.
//!
//! Re-captures each `seed_<n>/case.txt` a fuzz-loop run left behind (skipping seed
//! directories that hold only a `harness_failure.txt`), fresh against Chrome, and
//! canonicalises it with `parley_glyph_positioning_cases::minimise`. For every case that
//! still fails, this writes `minimised.txt` (a [`Golden`], seed preserved) and
//! `minimised_mismatch.txt` next to the input, then — once every input has been
//! processed — a batch `minimise_report.txt` at the output directory root, grouping
//! results that minimised to the same canonical content.
//!
//! ```sh
//! container/run.sh minimise [--out DIR] [case.txt ...]
//! ```
//!
//! or, against an already-running container (see `container/run.sh`):
//!
//! ```sh
//! cargo run -p parley_glyph_positioning_recorder --bin minimise -- [--out DIR] [case.txt ...]
//! ```
//!
//! With no explicit case paths, every `seed_*/case.txt` under `--out` (default
//! `target/glyph_positioning_fuzz`, matching `fuzz_loop`'s default) is processed, in
//! sorted order. Explicit paths are processed instead, in the order given (sorted), and
//! no report-directory discovery happens — the report, if any results are produced, is
//! still written under `--out`.
//!
//! `main` is deliberately sync, **not** `#[tokio::main]`: it owns a current-thread
//! [`tokio::runtime::Runtime`] and [`ChromeOracle`] borrows it, calling
//! `Runtime::block_on` per uncached capture — see `oracle`'s module docs for why
//! `Handle::block_on` would not work here.
//!
//! See "`src/bin/minimise.rs`" in `doc/glyph-positioning-chrome-parity-phase4.md` and
//! the minimiser design doc this implements.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use parley_glyph_positioning_cases::{
    Case, Golden, MinimiseError, MinimiseOutcome, case_content_key, minimise,
};
use parley_glyph_positioning_recorder::driver::{Config, Recorder};
use parley_glyph_positioning_recorder::oracle::ChromeOracle;
use parley_glyph_positioning_recorder::{Error, Result};

fn main() -> Result<()> {
    let args = Args::parse()?;
    let inputs = args.discover()?;
    if inputs.is_empty() {
        println!(
            "no case file(s) found under {} (or none given on the command line)",
            args.out.display()
        );
        return Ok(());
    }
    println!("minimising {} case file(s)", inputs.len());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let config = Config::from_env();
    let recorder = runtime.block_on(Recorder::attach(config.clone()))?;
    let mut oracle = ChromeOracle::new(&runtime, recorder, config);

    let mut outcomes: Vec<(PathBuf, MinimiseOutcome)> = Vec::new();
    let mut result = Ok(());
    for path in &inputs {
        match process_one(path, &mut oracle) {
            Ok(Some(outcome)) => outcomes.push((path.clone(), outcome)),
            Ok(None) => {}
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }

    // The out-dir discovery mode always gets a report, even an empty one, since it is
    // meant to be run unattended over a whole fuzz-loop batch; explicit-paths mode only
    // writes one if it actually produced something to group.
    let out_dir_mode = args.explicit.is_empty();
    if result.is_ok() && (out_dir_mode || !outcomes.is_empty()) {
        if let Err(error) = write_report(&args.out, &outcomes, &oracle) {
            eprintln!("failed writing minimise_report.txt: {error}");
        }
    } else if result.is_ok() {
        println!("no case(s) minimised; nothing to report");
    }

    let _ = runtime.block_on(oracle.into_recorder().close());
    result
}

/// Minimises the case in `path`, writing `minimised.txt`/`minimised_mismatch.txt`
/// alongside it on success.
///
/// - `Ok(Some(outcome))`: minimisation succeeded and was written.
/// - `Ok(None)`: the case is skipped (baseline now passes, or a non-fatal oracle
///   failure) — already logged to stderr.
/// - `Err(_)`: a fatal oracle failure (or an I/O error), which aborts the whole run.
fn process_one(path: &Path, oracle: &mut ChromeOracle<'_>) -> Result<Option<MinimiseOutcome>> {
    let text = std::fs::read_to_string(path)?;
    let golden = Golden::parse(&text)
        .map_err(|error| -> Error { format!("{}: {error}", path.display()).into() })?;

    match minimise(&golden.case, oracle) {
        Ok(outcome) => {
            write_outcome(path, &outcome, oracle)?;
            Ok(Some(outcome))
        }
        Err(MinimiseError::BaselinePasses) => {
            eprintln!(
                "{}: baseline now passes (fresh re-capture); skipping",
                path.display()
            );
            Ok(None)
        }
        Err(MinimiseError::Oracle(failure)) if failure.fatal => Err(format!(
            "{}: fatal oracle failure, aborting: {}",
            path.display(),
            failure.message
        )
        .into()),
        Err(MinimiseError::Oracle(failure)) => {
            eprintln!(
                "{}: oracle failure, skipping: {}",
                path.display(),
                failure.message
            );
            Ok(None)
        }
    }
}

/// Writes `minimised.txt` and `minimised_mismatch.txt` next to `path`, for a
/// successfully minimised `outcome`.
///
/// Written as soon as this one case is done (not batched to the end of the run), so a
/// killed run loses only the seed that was in flight.
fn write_outcome(
    path: &Path,
    outcome: &MinimiseOutcome,
    oracle: &mut ChromeOracle<'_>,
) -> Result<()> {
    // The minimiser's very last oracle call was evaluating exactly this final case, so
    // this is always a cache hit, not a fresh capture.
    let output = oracle.chrome_output(&outcome.case)?;

    let mut note = format!(
        "minimised: signature={} passes={} fixed_point={} tried={} skipped={}",
        outcome.signature,
        outcome.passes,
        outcome.reached_fixed_point,
        outcome.candidates_tried,
        outcome.skips.len(),
    );
    if !outcome.skips.is_empty() {
        write!(note, "; {}", outcome.skips.join("; ")).expect("writing to a String never fails");
    }
    // Defensive: `Golden::write` is a line-oriented format, and a skip message that
    // somehow still contained a newline would corrupt it.
    let note = single_line(&note);

    let golden = Golden {
        case: outcome.case.clone(),
        note: Some(note),
        output,
    };

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::write(dir.join("minimised.txt"), golden.write())?;
    std::fs::write(
        dir.join("minimised_mismatch.txt"),
        outcome.mismatch.to_string(),
    )?;
    Ok(())
}

/// Collapses `text` to one line, so a note field can never break [`Golden`]'s
/// line-oriented format no matter what an oracle's skip message contained.
fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Writes `<out>/minimise_report.txt`, grouping `outcomes` by [`case_content_key`], and
/// prints the same report to stdout.
fn write_report(
    out: &Path,
    outcomes: &[(PathBuf, MinimiseOutcome)],
    oracle: &ChromeOracle<'_>,
) -> Result<()> {
    let mut groups: BTreeMap<String, Vec<&(PathBuf, MinimiseOutcome)>> = BTreeMap::new();
    for entry in outcomes {
        groups
            .entry(case_content_key(&entry.1.case))
            .or_default()
            .push(entry);
    }

    let mut report = String::new();
    writeln!(
        report,
        "{} case(s) minimised into {} distinct canonical form(s)",
        outcomes.len(),
        groups.len()
    )
    .expect("writing to a String never fails");
    writeln!(report).expect("writing to a String never fails");

    for members in groups.values() {
        let mut seeds: Vec<u64> = members
            .iter()
            .map(|(_, outcome)| outcome.case.seed)
            .collect();
        seeds.sort_unstable();
        seeds.dedup();
        let representative = &members[0].1;
        let lowest_seed = seeds[0];

        writeln!(report, "signature: {}", representative.signature)
            .expect("writing to a String never fails");
        writeln!(
            report,
            "seeds: {}",
            seeds
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
        .expect("writing to a String never fails");
        writeln!(report, "case: {}", summarise_case(&representative.case))
            .expect("writing to a String never fails");
        writeln!(
            report,
            "promote to: parley_tests/tests/glyph_positioning/known_failing/minimised_seed_{lowest_seed}.txt"
        )
        .expect("writing to a String never fails");
        writeln!(report).expect("writing to a String never fails");
    }

    let total_tried: u64 = outcomes
        .iter()
        .map(|(_, outcome)| outcome.candidates_tried)
        .sum();
    writeln!(
        report,
        "stats: {} case(s) processed, {} capture(s), {} cache hit(s), {total_tried} candidate(s) tried",
        outcomes.len(),
        oracle.captures,
        oracle.cache_hits,
    )
    .expect("writing to a String never fails");

    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("minimise_report.txt"), &report)?;
    print!("{report}");
    Ok(())
}

/// A one-line human-readable summary of `case`'s content (everything but its `seed`),
/// for the report.
fn summarise_case(case: &Case) -> String {
    let runs = case
        .runs
        .iter()
        .map(|run| {
            format!(
                "(font_size={} letter_spacing={} word_spacing={} text={:?})",
                run.font_size, run.letter_spacing, run.word_spacing, run.text
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("width={} runs=[{runs}]", case.width)
}

/// Command-line configuration: an output directory (used both for out-dir discovery and
/// as the root the batch report is written under) and, optionally, explicit case file
/// paths to process instead of discovering them.
struct Args {
    out: PathBuf,
    explicit: Vec<PathBuf>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut out = PathBuf::from("target/glyph_positioning_fuzz");
        let mut explicit = Vec::new();

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => {
                    let value = args
                        .next()
                        .ok_or_else(|| -> Error { "--out needs a value".into() })?;
                    out = PathBuf::from(value);
                }
                other => explicit.push(PathBuf::from(other)),
            }
        }

        Ok(Self { out, explicit })
    }

    /// The case files to process, in sorted order.
    ///
    /// With explicit paths given on the command line, those are used as-is (sorted).
    /// Otherwise, every `seed_*/case.txt` directly under `self.out` is discovered,
    /// skipping any seed directory that holds only a `harness_failure.txt` (a harness
    /// failure, not a mismatch, has no case to minimise). A missing `self.out` is not an
    /// error: it just means there is nothing to do yet.
    fn discover(&self) -> Result<Vec<PathBuf>> {
        if !self.explicit.is_empty() {
            let mut paths = self.explicit.clone();
            paths.sort();
            return Ok(paths);
        }

        let mut paths = Vec::new();
        let entries = match std::fs::read_dir(&self.out) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let case_path = entry.path().join("case.txt");
            if case_path.is_file() {
                paths.push(case_path);
            }
        }
        paths.sort();
        Ok(paths)
    }
}
