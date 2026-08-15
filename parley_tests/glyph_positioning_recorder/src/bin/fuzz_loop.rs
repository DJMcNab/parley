// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! A long-running differential loop: generate a case, lay it out both ways, compare.
//!
//! One browser session is kept alive across many cases; it is recycled on error only.
//! Mismatches are written to a scratch directory and the loop continues — promotion into
//! the checked-in corpus is manual, mirroring `PARLEY_TEST=accept`.
//!
//! ```sh
//! container/run.sh fuzz_loop [--start-seed N] [--max-cases N] [--out DIR]
//! ```
//!
//! or, against an already-running container (see `container/run.sh`):
//!
//! ```sh
//! cargo run -p parley_glyph_positioning_recorder --bin fuzz_loop -- \
//!   [--start-seed N] [--max-cases N] [--out DIR]
//! ```
//!
//! See "`src/bin/fuzz_loop.rs`" in `doc/glyph-positioning-chrome-parity-phase4.md` and
//! `doc/glyph-positioning-recorder-agent.md`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parley::LayoutContext;
use parley_glyph_positioning_cases::{Case, Golden, compare};
use parley_glyph_positioning_extract::{font_context, layout, parley_output};
use parley_glyph_positioning_recorder::driver::{Config, Recorder, capture_with_retry};
use parley_glyph_positioning_recorder::{Error, Result};

/// The lowest seed the fuzz loop may use.
///
/// Disjoint from the golden corpus's range, so a fuzz hit can never be confused with a
/// checked-in case.
const MIN_FUZZ_SEED: u64 = 1_000_000;

/// How many consecutive harness failures abort the run.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Command-line configuration.
struct Args {
    start_seed: u64,
    max_cases: Option<u64>,
    out: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let args = Args::parse()?;
    std::fs::create_dir_all(&args.out)?;
    println!(
        "fuzzing from seed {} into {} (pin with --start-seed to reproduce)",
        args.start_seed,
        args.out.display()
    );

    let config = Config::from_env();
    let mut recorder = Recorder::attach(config.clone()).await?;
    let mut font_cx = font_context();
    let mut layout_cx = LayoutContext::new();

    let started = Instant::now();
    let mut cases = 0_u64;
    let mut mismatches = 0_u64;
    let mut harness_failures = 0_u64;
    let mut consecutive_failures = 0_u32;
    let mut result = Ok(());

    loop {
        if args.max_cases.is_some_and(|max| cases >= max) {
            break;
        }
        let seed = args.start_seed + cases;
        let case = Case::from_seed(seed);

        let recording = tokio::select! {
            recording = capture_with_retry(&mut recorder, &config, &case) => recording,
            _ = tokio::signal::ctrl_c() => {
                println!("interrupted");
                break;
            }
        };

        cases += 1;
        let recording = match recording {
            Ok(recording) => {
                consecutive_failures = 0;
                recording
            }
            Err(error) => {
                harness_failures += 1;
                consecutive_failures += 1;
                eprintln!("seed {seed}: harness failure: {error}");
                write_artifact(&args.out, seed, |dir| {
                    std::fs::write(dir.join("harness_failure.txt"), error.to_string())?;
                    Ok(())
                })?;
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    result = Err(format!(
                        "aborting after {consecutive_failures} consecutive harness failures; the \
                         container or the WebDriver session is not healthy"
                    )
                    .into());
                    break;
                }
                continue;
            }
        };

        let laid_out = layout(&case, &mut font_cx, &mut layout_cx);
        let parley = parley_output(&laid_out);
        if let Err(mismatch) = compare(&parley, &recording.output) {
            mismatches += 1;
            eprintln!("seed {seed}: mismatch\n{mismatch}");
            let golden = Golden {
                case: case.clone(),
                note: None,
                output: recording.output,
            };
            // The raw dump and the `.skp` are kept because a deserializer bug and a
            // genuine parity bug look identical in the position diff alone.
            write_artifact(&args.out, seed, |dir| {
                std::fs::write(dir.join("case.txt"), golden.write())?;
                std::fs::write(dir.join("mismatch.txt"), mismatch.to_string())?;
                std::fs::write(dir.join("capture.json"), &recording.json)?;
                std::fs::write(dir.join("capture.skp"), &recording.skp_bytes)?;
                Ok(())
            })?;
        }
    }

    let _ = recorder.close().await;
    report(started.elapsed(), cases, mismatches, harness_failures);
    result
}

/// Runs `write` against a fresh (or reused) per-seed artifact directory.
fn write_artifact(
    out: &Path,
    seed: u64,
    write: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<()> {
    let dir = out.join(format!("seed_{seed}"));
    std::fs::create_dir_all(&dir)?;
    write(&dir)?;
    println!("artifact: {}", dir.display());
    Ok(())
}

fn report(elapsed: Duration, cases: u64, mismatches: u64, harness_failures: u64) {
    let rate = cases as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    println!(
        "{cases} case(s) in {:.1}s ({rate:.2}/s): {mismatches} mismatch(es), \
         {harness_failures} harness failure(s)",
        elapsed.as_secs_f64()
    );
}

impl Args {
    fn parse() -> Result<Self> {
        let mut start_seed = None;
        let mut max_cases = None;
        let mut out = PathBuf::from("target/glyph_positioning_fuzz");

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || {
                args.next()
                    .ok_or_else(|| -> Error { format!("{arg} needs a value").into() })
            };
            match arg.as_str() {
                "--start-seed" => start_seed = Some(value()?.parse::<u64>()?),
                "--max-cases" => max_cases = Some(value()?.parse::<u64>()?),
                "--out" => out = PathBuf::from(value()?),
                other => return Err(format!("unrecognised argument {other:?}").into()),
            }
        }

        let start_seed = start_seed.unwrap_or_else(default_start_seed);
        if start_seed < MIN_FUZZ_SEED {
            return Err(format!(
                "--start-seed must be at least {MIN_FUZZ_SEED}, so fuzz seeds stay disjoint from \
                 the golden corpus"
            )
            .into());
        }
        Ok(Self {
            start_seed,
            max_cases,
            out,
        })
    }
}

/// A wall-clock-derived seed base, so successive runs explore different cases.
///
/// Derived from the clock rather than from `rand` — the loop needs no randomness of its
/// own, only a starting point — and always logged, so any hit is reproducible with
/// `--start-seed`.
fn default_start_seed() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    MIN_FUZZ_SEED + u64::from(since_epoch.subsec_nanos()) + since_epoch.as_secs() * 1_000_000_000
}
