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
//! container/run.sh minimise [--jobs N] [--force] [--out DIR] [case.txt ...]
//! ```
//!
//! or, against an already-running container (see `container/run.sh`):
//!
//! ```sh
//! cargo run -p parley_glyph_positioning_recorder --bin minimise -- \
//!     [--jobs N] [--force] [--out DIR] [case.txt ...]
//! ```
//!
//! With no explicit case paths, every `seed_*/case.txt` under `--out` (default
//! `target/glyph_positioning_fuzz`, matching `fuzz_loop`'s default) is processed, in
//! sorted order. A discovered seed with both minimisation artifacts already present is
//! loaded into the report without opening Chrome; `--force` reprocesses it. Explicit
//! paths are always processed instead, in the order given (sorted), and no
//! report-directory discovery happens — the report, if any results are produced, is
//! still written under `--out`.
//!
//! `--jobs` (default 1) minimises independent inputs on separate long-lived Chrome
//! sessions in the same container. A dynamic queue balances the long tail. Each worker
//! owns a current-thread [`tokio::runtime::Runtime`], and its [`ChromeOracle`] borrows
//! it, calling `Runtime::block_on` per uncached capture.
//!
//! See "`src/bin/minimise.rs`" in `doc/glyph-positioning-chrome-parity-phase4.md` and
//! the minimiser design doc this implements.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use parley_glyph_positioning_cases::{
    Case, Golden, MinimiseError, MinimiseOutcome, MinimisePhaseStats, MinimiseStats,
    case_content_key, minimise,
};
use parley_glyph_positioning_recorder::driver::{Config, Recorder};
use parley_glyph_positioning_recorder::oracle::ChromeOracle;
use parley_glyph_positioning_recorder::{Error, Result};

fn main() -> Result<()> {
    let args = Args::parse()?;
    let discovery = args.discover()?;
    if discovery.pending.is_empty() && discovery.resumed.is_empty() {
        println!(
            "no case file(s) found under {} (or none given on the command line)",
            args.out.display()
        );
        return Ok(());
    }
    if discovery.pending.is_empty() {
        println!(
            "all {} discovered case(s) already have complete minimisation artifacts",
            discovery.resumed.len()
        );
        write_report(
            &args.out,
            &discovery.resumed,
            ReportStats {
                resumed: discovery.resumed.len(),
                ..ReportStats::default()
            },
        )?;
        return Ok(());
    }
    let jobs = args.jobs.min(discovery.pending.len());
    println!(
        "minimising {} case file(s) with {jobs} Chrome worker(s) ({} resumed)",
        discovery.pending.len(),
        discovery.resumed.len(),
    );

    let config = Config::from_env();
    let queue = Arc::new(Mutex::new(VecDeque::from(discovery.pending)));
    let worker_results: Vec<WorkerResult> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(jobs);
        for worker_index in 0..jobs {
            let queue = Arc::clone(&queue);
            let config = config.clone();
            handles.push(scope.spawn(move || run_worker(worker_index, queue, config)));
        }

        let mut results = Vec::with_capacity(jobs);
        for handle in handles {
            let result = handle
                .join()
                .map_err(|_| -> Error { "a minimiser worker panicked".into() })??;
            results.push(result);
        }
        Ok::<_, Error>(results)
    })?;

    let mut outcomes = Vec::new();
    let mut stats = OracleStats::default();
    for worker in worker_results {
        outcomes.extend(worker.outcomes);
        stats.captures += worker.stats.captures;
        stats.cache_hits += worker.stats.cache_hits;
    }
    outcomes.sort_by(|left, right| left.0.cmp(&right.0));
    let mut entries = discovery.resumed;
    entries.extend(outcomes.iter().map(|(path, outcome)| ReportEntry {
        path: path.clone(),
        case: outcome.case.clone(),
        signature: outcome.signature.to_string(),
    }));
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let report_stats = ReportStats {
        processed: outcomes.len(),
        resumed: entries.len().saturating_sub(outcomes.len()),
        oracle: stats,
        candidates: outcomes
            .iter()
            .map(|(_, outcome)| outcome.candidates_tried)
            .sum(),
        phases: outcomes
            .iter()
            .fold(MinimiseStats::default(), |mut total, entry| {
                add_minimise_stats(&mut total, &entry.1.stats);
                total
            }),
    };

    // The out-dir discovery mode always gets a report, even an empty one, since it is
    // meant to be run unattended over a whole fuzz-loop batch; explicit-paths mode only
    // writes one if it actually produced something to group.
    let out_dir_mode = args.explicit.is_empty();
    if out_dir_mode || !entries.is_empty() {
        if let Err(error) = write_report(&args.out, &entries, report_stats) {
            eprintln!("failed writing minimise_report.txt: {error}");
        }
    } else {
        println!("no case(s) minimised; nothing to report");
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct OracleStats {
    captures: u64,
    cache_hits: u64,
}

#[derive(Default)]
struct ReportStats {
    processed: usize,
    resumed: usize,
    oracle: OracleStats,
    candidates: u64,
    phases: MinimiseStats,
}

struct ReportEntry {
    path: PathBuf,
    case: Case,
    signature: String,
}

struct Discovery {
    pending: Vec<PathBuf>,
    resumed: Vec<ReportEntry>,
}

struct WorkerResult {
    outcomes: Vec<(PathBuf, MinimiseOutcome)>,
    stats: OracleStats,
}

fn run_worker(
    worker_index: usize,
    queue: Arc<Mutex<VecDeque<PathBuf>>>,
    config: Config,
) -> Result<WorkerResult> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let recorder = runtime.block_on(Recorder::attach(config.clone()))?;
    let mut oracle = ChromeOracle::new(&runtime, recorder, config);
    let mut outcomes = Vec::new();
    let mut result = Ok(());

    loop {
        let path = queue
            .lock()
            .expect("minimiser work queue mutex poisoned")
            .pop_front();
        let Some(path) = path else { break };
        println!("worker {}: {}", worker_index + 1, path.display());
        match process_one(&path, &mut oracle) {
            Ok(Some(outcome)) => outcomes.push((path, outcome)),
            Ok(None) => {}
            Err(error) => {
                result = Err(error);
                break;
            }
        }
    }

    let stats = OracleStats {
        captures: oracle.captures,
        cache_hits: oracle.cache_hits,
    };
    let close_result = runtime.block_on(oracle.into_recorder().close());
    result?;
    close_result?;
    Ok(WorkerResult { outcomes, stats })
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
fn write_report(out: &Path, outcomes: &[ReportEntry], stats: ReportStats) -> Result<()> {
    let mut groups: BTreeMap<String, Vec<&ReportEntry>> = BTreeMap::new();
    for entry in outcomes {
        groups
            .entry(case_content_key(&entry.case))
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
        let mut seeds: Vec<u64> = members.iter().map(|entry| entry.case.seed).collect();
        seeds.sort_unstable();
        seeds.dedup();
        let representative = members[0];
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

    writeln!(
        report,
        "stats: {} case(s) in report: {} processed now, {} resumed; {} capture(s), {} cache hit(s), {} candidate(s) tried",
        outcomes.len(), stats.processed, stats.resumed, stats.oracle.captures,
        stats.oracle.cache_hits, stats.candidates,
    )
    .expect("writing to a String never fails");
    if stats.processed != 0 {
        writeln!(report, "phase stats (candidates / captures / cache hits):")
            .expect("writing to a String never fails");
        for (name, phase) in phase_rows(&stats.phases) {
            writeln!(
                report,
                "  {name}: {} / {} / {}",
                phase.candidates, phase.captures, phase.cache_hits
            )
            .expect("writing to a String never fails");
        }
    }

    std::fs::create_dir_all(out)?;
    std::fs::write(out.join("minimise_report.txt"), &report)?;
    print!("{report}");
    Ok(())
}

fn add_phase(total: &mut MinimisePhaseStats, value: &MinimisePhaseStats) {
    total.candidates += value.candidates;
    total.captures += value.captures;
    total.cache_hits += value.cache_hits;
}

fn add_minimise_stats(total: &mut MinimiseStats, value: &MinimiseStats) {
    add_phase(&mut total.baseline, &value.baseline);
    add_phase(&mut total.width_exact, &value.width_exact);
    add_phase(&mut total.scalar_exacts, &value.scalar_exacts);
    add_phase(&mut total.remove_runs, &value.remove_runs);
    add_phase(&mut total.ddmin_chars, &value.ddmin_chars);
    add_phase(&mut total.scalar_ladder, &value.scalar_ladder);
    add_phase(&mut total.char_scan, &value.char_scan);
}

fn phase_rows(stats: &MinimiseStats) -> [(&'static str, &MinimisePhaseStats); 7] {
    [
        ("baseline", &stats.baseline),
        ("width exact", &stats.width_exact),
        ("scalar exacts", &stats.scalar_exacts),
        ("remove runs", &stats.remove_runs),
        ("ddmin chars", &stats.ddmin_chars),
        ("scalar ladder", &stats.scalar_ladder),
        ("char scan", &stats.char_scan),
    ]
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
    jobs: usize,
    force: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut out = PathBuf::from("target/glyph_positioning_fuzz");
        let mut explicit = Vec::new();
        let mut jobs = 1;
        let mut force = false;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => {
                    let value = args
                        .next()
                        .ok_or_else(|| -> Error { "--out needs a value".into() })?;
                    out = PathBuf::from(value);
                }
                "--jobs" => {
                    let value = args
                        .next()
                        .ok_or_else(|| -> Error { "--jobs needs a value".into() })?;
                    jobs = value.parse().map_err(|_| -> Error {
                        format!("--jobs must be a positive integer, got {value:?}").into()
                    })?;
                    if jobs == 0 {
                        return Err("--jobs must be at least 1".into());
                    }
                }
                "--force" => force = true,
                other => explicit.push(PathBuf::from(other)),
            }
        }

        Ok(Self {
            out,
            explicit,
            jobs,
            force,
        })
    }

    /// The case files to process, in sorted order.
    ///
    /// With explicit paths given on the command line, those are used as-is (sorted).
    /// Otherwise, every `seed_*/case.txt` directly under `self.out` is discovered,
    /// skipping any seed directory that holds only a `harness_failure.txt` (a harness
    /// failure, not a mismatch, has no case to minimise). A missing `self.out` is not an
    /// error: it just means there is nothing to do yet.
    fn discover(&self) -> Result<Discovery> {
        if !self.explicit.is_empty() {
            let mut paths = self.explicit.clone();
            paths.sort();
            return Ok(Discovery {
                pending: paths,
                resumed: Vec::new(),
            });
        }

        let mut paths = Vec::new();
        let entries = match std::fs::read_dir(&self.out) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Discovery {
                    pending: paths,
                    resumed: Vec::new(),
                });
            }
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
        let mut pending = Vec::new();
        let mut resumed = Vec::new();
        for path in paths {
            let dir = path.parent().unwrap_or_else(|| Path::new("."));
            let minimised = dir.join("minimised.txt");
            let mismatch = dir.join("minimised_mismatch.txt");
            if !self.force && minimised.is_file() && mismatch.is_file() {
                resumed.push(load_report_entry(&path, &minimised)?);
            } else {
                pending.push(path);
            }
        }
        Ok(Discovery { pending, resumed })
    }
}

fn load_report_entry(source: &Path, minimised: &Path) -> Result<ReportEntry> {
    let text = std::fs::read_to_string(minimised)?;
    let golden = Golden::parse(&text)
        .map_err(|error| -> Error { format!("{}: {error}", minimised.display()).into() })?;
    let note = golden.note.as_deref().ok_or_else(|| -> Error {
        format!("{}: missing minimiser note", minimised.display()).into()
    })?;
    let signature = note
        .split_whitespace()
        .find_map(|token| token.strip_prefix("signature="))
        .filter(|signature| !signature.is_empty())
        .ok_or_else(|| -> Error {
            format!("{}: minimiser note has no signature", minimised.display()).into()
        })?;
    Ok(ReportEntry {
        path: source.to_path_buf(),
        case: golden.case,
        signature: signature.to_string(),
    })
}
