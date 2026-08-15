// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Signature-preserving minimisation of a failing [`Case`].
//!
//! Given a [`Case`] that an [`Oracle`] reports as failing, [`minimise`] repeatedly
//! shrinks it — dropping the width to the trivial single-line value, snapping scalars to
//! their exact targets, deleting characters, and replacing characters with earlier
//! alphabet entries — accepting each shrink only if the case still fails with the same
//! [`FailureSignature`]. This produces a small, deterministic **canonical** form for a
//! failure without needing to know *why* it fails: the signature check is what keeps a
//! shrink from wandering into an unrelated bug.
//!
//! The oracle is intentionally sync and generic (a plain closure works, via the blanket
//! [`Oracle`] impl) so this whole engine is unit-testable without Chrome. The
//! Chrome-backed oracle and CLI live in `parley_glyph_positioning_recorder`, which is
//! native-only.
//!
//! # Determinism
//!
//! Every rule iterates in a fixed order over a fixed enumeration (run index, then
//! character position, then alphabet index), with no RNG and no wall-clock dependency.
//! Given the same case and an oracle that answers deterministically, [`minimise`]
//! produces the same [`MinimiseOutcome`] every time.
//!
//! # Candidate validity
//!
//! Every candidate [`Case`] a rule considers is checked against
//! [`is_valid_candidate`] *before* it is ever shown to the oracle: an invalid candidate
//! is rejected locally, without spending an oracle call. This mirrors the invariants
//! [`Case::from_seed`] guarantees for a generated case (see "The `Case` grammar" in
//! `doc/glyph-positioning-chrome-parity-phase1.md`), so a minimised case remains
//! something the harness — and a human reading it — can trust follows the same
//! grammar as every other case.

use std::fmt::Write as _;

use crate::chromium_quantization::{LAYOUT_UNIT_STEPS_PER_PX, SPACING_GRID_STEPS_PER_PX};
use crate::compare::Mismatch;
use crate::generate::{
    Case, FONT_SIZE_STEP, MAX_CASE_WIDTH_PX, MAX_FONT_SIZE, MIN_FONT_SIZE, alphabet,
    valid_font_size,
};
use crate::signature::FailureSignature;

/// The number of grid steps per CSS px [`sample_font_size`](crate::generate)'s
/// `FONT_SIZE_STEP` grid uses. Kept local (rather than re-deriving `1.0 /
/// FONT_SIZE_STEP` at every call site) purely for readability at the ladder call sites
/// in [`Minimiser::scalar_ladder`].
const FONT_SIZE_STEPS_PER_PX: f32 = 1.0 / FONT_SIZE_STEP;

/// The maximum number of shrink passes [`minimise`] runs before stopping even if the
/// case is still changing. Every accepted step strictly shrinks something (text length,
/// a scalar's grid-step distance to its target, or a character's alphabet index), so in
/// practice a fixed point is reached in far fewer passes than this; it exists as a
/// backstop against a rule ordering that could in principle keep unlocking new shrinks
/// of each other indefinitely.
const MAX_PASSES: u32 = 5;

/// The number of alphabet candidates scanned per character position in the min-first
/// scan ([`Minimiser::char_scan`]).
///
/// Must be large enough to clear Roboto's combining-mark range: [`alphabet`] is sorted
/// by codepoint, and the ASCII + Latin-1 + Latin Extended-A/B prefix below U+0300 (where
/// Roboto's combining marks begin) occupies roughly 400-500 of its entries. 600 gives
/// comfortable margin over that; re-derive this bound if the bundled font
/// ([`crate::FONTS`]) changes.
const CHAR_SCAN_CAP: usize = 600;

/// Why an [`Oracle`] could not evaluate a candidate case at all.
#[derive(Clone, Debug)]
pub struct OracleFailure {
    /// A human-readable description of the failure, for skip/abort reporting.
    pub message: String,
    /// Whether this failure should abort minimisation outright (`Err`), rather than
    /// being recorded as a skip and continuing with the next candidate.
    pub fatal: bool,
}

/// Evaluates whether a [`Case`] reproduces the failure being minimised.
///
/// A blanket implementation covers any `FnMut(&Case) -> Result<Result<(), Mismatch>,
/// OracleFailure>` closure, which is all the engine's own unit tests use; the
/// Chrome-backed implementation lives in `parley_glyph_positioning_recorder`, which
/// wraps captures and comparison behind this same interface.
pub trait Oracle {
    /// Evaluates `case`.
    ///
    /// - `Ok(Ok(()))`: the case passes — it does not reproduce the failure.
    /// - `Ok(Err(mismatch))`: the case fails with `mismatch`.
    /// - `Err(failure)`: the case could not be evaluated (e.g. a transient capture
    ///   error). The candidate is skipped unless [`failure.fatal`](OracleFailure::fatal)
    ///   is set, in which case minimisation aborts.
    fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure>;
}

impl<F> Oracle for F
where
    F: FnMut(&Case) -> Result<Result<(), Mismatch>, OracleFailure>,
{
    fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure> {
        self(case)
    }
}

/// Why [`minimise`] could not produce a [`MinimiseOutcome`].
#[derive(Debug)]
pub enum MinimiseError {
    /// The original case already passes: there is nothing to minimise.
    BaselinePasses,
    /// The oracle could not evaluate the original case at all, or a later fatal
    /// [`OracleFailure`] aborted minimisation.
    Oracle(OracleFailure),
}

/// The result of successfully minimising a failing [`Case`].
#[derive(Debug)]
pub struct MinimiseOutcome {
    /// The minimised case. Its `seed` is carried over from the original case unchanged
    /// — provenance only, never used to regenerate it.
    pub case: Case,
    /// The [`FailureSignature`] every accepted shrink preserved, computed from a fresh
    /// evaluation of the original case.
    pub signature: FailureSignature,
    /// The final case's mismatch, from the oracle evaluation that most recently
    /// accepted it.
    pub mismatch: Mismatch,
    /// How many shrink passes ran before reaching a fixed point or [`MAX_PASSES`].
    pub passes: u32,
    /// Whether minimisation reached a true fixed point (the last pass made no further
    /// change), as opposed to being cut off by the [`MAX_PASSES`] cap while still
    /// changing.
    pub reached_fixed_point: bool,
    /// The total number of candidates evaluated by the oracle (successful, failed, and
    /// skipped alike).
    pub candidates_tried: u64,
    /// Terse, single-line messages for candidates the oracle could not evaluate
    /// (non-fatal [`OracleFailure`]s), in the order they occurred.
    pub skips: Vec<String>,
}

/// Minimises `case` against `oracle`, shrinking it while preserving its
/// [`FailureSignature`]. See the module docs for the rules this applies, in order, each
/// pass.
///
/// The very first thing this does is re-evaluate `case` itself, fresh, to establish the
/// baseline signature — a checked-in `capture.json` may predate a harness or Chrome
/// change, so the signature to preserve always comes from a live re-capture rather than
/// from whatever originally flagged this case as failing.
pub fn minimise(case: &Case, oracle: &mut dyn Oracle) -> Result<MinimiseOutcome, MinimiseError> {
    let baseline_mismatch = match oracle.evaluate(case).map_err(MinimiseError::Oracle)? {
        Ok(()) => return Err(MinimiseError::BaselinePasses),
        Err(mismatch) => mismatch,
    };
    let baseline = FailureSignature::of(&baseline_mismatch);
    let font_size_target = target_font_size();

    let mut minimiser = Minimiser {
        oracle,
        baseline,
        best_mismatch: baseline_mismatch,
        skips: Vec::new(),
        candidates_tried: 0,
    };

    let mut working = case.clone();
    let mut passes = 0_u32;
    let mut changed = true;
    while changed && passes < MAX_PASSES {
        let before = working.clone();

        minimiser.width_exact(&mut working)?;
        minimiser.scalar_exacts(&mut working, font_size_target)?;
        minimiser.remove_runs(&mut working)?;
        minimiser.ddmin_chars(&mut working)?;
        minimiser.scalar_ladder(&mut working, font_size_target)?;
        minimiser.char_scan(&mut working)?;

        changed = working != before;
        passes += 1;
    }

    Ok(MinimiseOutcome {
        case: working,
        signature: minimiser.baseline,
        mismatch: minimiser.best_mismatch,
        passes,
        reached_fixed_point: !changed,
        candidates_tried: minimiser.candidates_tried,
        skips: minimiser.skips,
    })
}

/// A deterministic content key for `case`, excluding its `seed`: the width plus, per
/// run, `(font_size, letter_spacing, word_spacing, line_height, text)`. Floats are
/// rendered via
/// `Display` (Rust's shortest round-tripping decimal), so two cases with the same
/// content but different provenance produce the same key.
///
/// Used to cache oracle evaluations by content within a minimisation batch, and to
/// group minimised cases for dedup reporting — both defined in
/// `parley_glyph_positioning_recorder`, which is the only thing that needs this key to
/// be *anything* in particular beyond deterministic and content-addressed.
#[must_use]
pub fn case_content_key(case: &Case) -> String {
    let mut key = format!("w={}", case.width);
    for run in &case.runs {
        write!(
            key,
            "|fs={} ls={} ws={} lh={} t={}",
            run.font_size, run.letter_spacing, run.word_spacing, run.line_height, run.text
        )
        .expect("writing to a String never fails");
    }
    key
}

/// Whether `case` is a legal minimisation candidate: it satisfies every grid, range,
/// and alphabet invariant [`Case::from_seed`] guarantees for a generated case. Every
/// rule checks this before ever showing a candidate to the oracle.
///
/// `pub(crate)` (rather than private) purely so this module's own tests can assert that
/// every candidate a rule produces is valid, in addition to wrapping the fake oracle to
/// assert the same thing from the other side.
pub(crate) fn is_valid_candidate(case: &Case) -> bool {
    if case.runs.is_empty() {
        return false;
    }
    if !(case.width > 0.0
        && case.width <= MAX_CASE_WIDTH_PX
        && crate::chromium_quantization::is_on_grid(case.width, LAYOUT_UNIT_STEPS_PER_PX))
    {
        return false;
    }

    let alphabet = alphabet();
    // No run may start with a space (matching `generate_run_text`'s `at_run_start`
    // check, which forbids it unconditionally); no two spaces may be adjacent,
    // including across a run boundary — tracked the same way generation tracks it, by
    // carrying `prev_was_space` from one run's last character into the next run's
    // first.
    let mut prev_was_space = true;
    let mut has_non_space = false;
    for run in &case.runs {
        if run.text.is_empty()
            || run.text.starts_with(' ')
            || !valid_font_size(run.font_size)
            || !crate::chromium_quantization::is_on_grid(
                run.letter_spacing,
                SPACING_GRID_STEPS_PER_PX,
            )
            || !crate::chromium_quantization::is_on_grid(
                run.word_spacing,
                SPACING_GRID_STEPS_PER_PX,
            )
            || run.line_height <= 0.0
            || !crate::chromium_quantization::is_on_grid(run.line_height, LAYOUT_UNIT_STEPS_PER_PX)
        {
            return false;
        }
        for ch in run.text.chars() {
            if ch == ' ' {
                if prev_was_space {
                    return false;
                }
            } else if alphabet.binary_search(&ch).is_err() {
                return false;
            } else {
                has_non_space = true;
            }
            prev_was_space = ch == ' ';
        }
    }
    has_non_space
}

/// The value the scalar-exact and ladder rules shrink every run's `font_size` toward:
/// `16.0` if that passes [`valid_font_size`] for the bundled font, else the nearest
/// [`FONT_SIZE_STEP`]-grid value in range that does, tie-breaking toward the lower
/// value.
///
/// Computed once per [`minimise`] call rather than hardcoded, so this keeps working if
/// the bundled font ever changes to one for which 16px trips the `FreeType` hack.
fn target_font_size() -> f32 {
    const PREFERRED: f32 = 16.0;
    if valid_font_size(PREFERRED) {
        return PREFERRED;
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the sampled font-size range has far fewer than i32::MAX grid steps"
    )]
    let total_steps = ((MAX_FONT_SIZE - MIN_FONT_SIZE) / FONT_SIZE_STEP).round() as i32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the sampled font-size range has far fewer than i32::MAX grid steps"
    )]
    let preferred_step = ((PREFERRED - MIN_FONT_SIZE) / FONT_SIZE_STEP).round() as i32;

    (0..=total_steps)
        .map(|step| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "step counts here stay under 100, far inside f32's exact-integer range"
            )]
            let size = MIN_FONT_SIZE + step as f32 * FONT_SIZE_STEP;
            (step, size)
        })
        .filter(|&(_, size)| valid_font_size(size))
        .min_by_key(|&(step, _)| ((step - preferred_step).unsigned_abs(), step))
        .map(|(_, size)| size)
        .expect(
            "at least one grid value in [MIN_FONT_SIZE, MAX_FONT_SIZE] doesn't trip the \
             FreeType hack",
        )
}

/// Collapses `message` to a single line with no repeated whitespace, so a skip record
/// stays one line no matter what an oracle's failure message looked like.
fn terse(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drives one [`minimise`] call: owns the oracle, the signature every accepted
/// candidate must preserve, and the running stats. Its rule methods (one per lettered
/// rule in the module docs) each take the working [`Case`] and mutate it in place on
/// acceptance.
struct Minimiser<'o> {
    oracle: &'o mut dyn Oracle,
    baseline: FailureSignature,
    best_mismatch: Mismatch,
    skips: Vec<String>,
    candidates_tried: u64,
}

impl Minimiser<'_> {
    /// Evaluates `candidate` against the oracle, WITHOUT checking [`is_valid_candidate`]
    /// first — callers must do that themselves; see [`Self::try_candidate`] for the
    /// combined check-then-evaluate most rules want.
    ///
    /// Returns whether `candidate` preserves [`Self::baseline`]: `Ok(true)` on a
    /// signature-preserving failure (also updating [`Self::best_mismatch`]), `Ok(false)`
    /// if it passes or fails with a different signature, and `Err` if a fatal
    /// [`OracleFailure`] should abort minimisation (a non-fatal one is recorded in
    /// [`Self::skips`] and treated as `Ok(false)`).
    fn preserves(&mut self, candidate: &Case) -> Result<bool, MinimiseError> {
        self.candidates_tried += 1;
        match self.oracle.evaluate(candidate) {
            Err(failure) => {
                if failure.fatal {
                    Err(MinimiseError::Oracle(failure))
                } else {
                    self.skips.push(terse(&failure.message));
                    Ok(false)
                }
            }
            Ok(Ok(())) => Ok(false),
            Ok(Err(mismatch)) => {
                if FailureSignature::of(&mismatch) == self.baseline {
                    self.best_mismatch = mismatch;
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Checks [`is_valid_candidate`] before spending an oracle call, then
    /// [`Self::preserves`]. Returns `candidate` back if it was valid and
    /// signature-preserving.
    fn try_candidate(&mut self, candidate: Case) -> Result<Option<Case>, MinimiseError> {
        if !is_valid_candidate(&candidate) {
            return Ok(None);
        }
        if self.preserves(&candidate)? {
            Ok(Some(candidate))
        } else {
            Ok(None)
        }
    }

    /// Rule (a): tries `working.width == MAX_CASE_WIDTH_PX` exactly.
    fn width_exact(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        if working.width != MAX_CASE_WIDTH_PX {
            let mut candidate = working.clone();
            candidate.width = MAX_CASE_WIDTH_PX;
            if let Some(accepted) = self.try_candidate(candidate)? {
                *working = accepted;
            }
        }
        Ok(())
    }

    /// Rule (b): tries each run's `font_size -> font_size_target`, `letter_spacing ->
    /// 0.0`, and `word_spacing -> 0.0` exactly, one candidate per `(run, field)` that
    /// differs from its target.
    ///
    /// Does not touch `line_height` — there is no obvious "nicest" absolute-px target
    /// to snap it to (unlike `0.0` for the spacings), so a minimised case keeps
    /// whatever `line_height` its originating seed sampled. [`is_valid_candidate`]
    /// still checks it stays on-grid and positive, since no rule here ever changes it
    /// away from a value [`Case::from_seed`] already guaranteed valid.
    fn scalar_exacts(
        &mut self,
        working: &mut Case,
        font_size_target: f32,
    ) -> Result<(), MinimiseError> {
        for index in 0..working.runs.len() {
            if working.runs[index].font_size != font_size_target {
                let mut candidate = working.clone();
                candidate.runs[index].font_size = font_size_target;
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                }
            }
        }
        for index in 0..working.runs.len() {
            if working.runs[index].letter_spacing != 0.0 {
                let mut candidate = working.clone();
                candidate.runs[index].letter_spacing = 0.0;
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                }
            }
        }
        for index in 0..working.runs.len() {
            if working.runs[index].word_spacing != 0.0 {
                let mut candidate = working.clone();
                candidate.runs[index].word_spacing = 0.0;
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                }
            }
        }
        Ok(())
    }

    /// Rule (c): tries deleting each run outright, ascending by index. A no-op today
    /// (`MAX_RUNS` is pinned to 1, so there is never a second run to fall back to), but
    /// implemented so it starts working the moment that pin lifts.
    fn remove_runs(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        let mut index = 0;
        while index < working.runs.len() {
            if working.runs.len() > 1 {
                let mut candidate = working.clone();
                candidate.runs.remove(index);
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                    // A later run just shifted into `index`; try it too before
                    // advancing.
                    continue;
                }
            }
            index += 1;
        }
        Ok(())
    }

    /// Rule (d): chunked ddmin-style character deletion, per run (ascending), on
    /// `Vec<char>` (never byte-indexed, since a run's text can contain non-ASCII
    /// alphabet entries).
    fn ddmin_chars(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        for run_index in 0..working.runs.len() {
            self.ddmin_run(working, run_index)?;
        }
        Ok(())
    }

    /// The per-run body of [`Self::ddmin_chars`]: for chunk sizes `ceil(len/2)`, halved
    /// each round down to `1`, slides a deletion window across the run's characters,
    /// keeping any deletion that stays valid and signature-preserving without advancing
    /// past it (so an accepted deletion is immediately retried at the same position).
    fn ddmin_run(&mut self, working: &mut Case, run_index: usize) -> Result<(), MinimiseError> {
        let mut chars: Vec<char> = working.runs[run_index].text.chars().collect();
        let mut size = chars.len().div_ceil(2);
        while size >= 1 {
            let mut i = 0;
            while i + size <= chars.len() {
                let mut candidate_chars = chars.clone();
                candidate_chars.drain(i..i + size);
                let mut candidate = working.clone();
                candidate.runs[run_index].text = candidate_chars.iter().collect();
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                    chars = candidate_chars;
                } else {
                    i += size;
                }
            }
            if size == 1 {
                break;
            }
            size = size.div_ceil(2);
        }
        Ok(())
    }

    /// Rule (e): for every scalar not already at its target after rules (a)-(b), runs
    /// the grid-quantized [`Self::ladder`] in the settled order: `width`, then each
    /// run's `font_size`, `letter_spacing`, and `word_spacing`.
    fn scalar_ladder(
        &mut self,
        working: &mut Case,
        font_size_target: f32,
    ) -> Result<(), MinimiseError> {
        if working.width != MAX_CASE_WIDTH_PX {
            let base = working.clone();
            let new_width = self.ladder(
                working.width,
                MAX_CASE_WIDTH_PX,
                LAYOUT_UNIT_STEPS_PER_PX,
                |value| {
                    let mut candidate = base.clone();
                    candidate.width = value;
                    candidate
                },
            )?;
            working.width = new_width;
        }

        for index in 0..working.runs.len() {
            if working.runs[index].font_size == font_size_target {
                continue;
            }
            let base = working.clone();
            let new_value = self.ladder(
                working.runs[index].font_size,
                font_size_target,
                FONT_SIZE_STEPS_PER_PX,
                |value| {
                    let mut candidate = base.clone();
                    candidate.runs[index].font_size = value;
                    candidate
                },
            )?;
            working.runs[index].font_size = new_value;
        }

        for index in 0..working.runs.len() {
            if working.runs[index].letter_spacing == 0.0 {
                continue;
            }
            let base = working.clone();
            let new_value = self.ladder(
                working.runs[index].letter_spacing,
                0.0,
                SPACING_GRID_STEPS_PER_PX,
                |value| {
                    let mut candidate = base.clone();
                    candidate.runs[index].letter_spacing = value;
                    candidate
                },
            )?;
            working.runs[index].letter_spacing = new_value;
        }

        for index in 0..working.runs.len() {
            if working.runs[index].word_spacing == 0.0 {
                continue;
            }
            let base = working.clone();
            let new_value = self.ladder(
                working.runs[index].word_spacing,
                0.0,
                SPACING_GRID_STEPS_PER_PX,
                |value| {
                    let mut candidate = base.clone();
                    candidate.runs[index].word_spacing = value;
                    candidate
                },
            )?;
            working.runs[index].word_spacing = new_value;
        }

        Ok(())
    }

    /// The generic two-stage scalar ladder shared by every [`Self::scalar_ladder`] call:
    /// shrinks `current` toward `target` on a grid of `steps_per_px` steps per CSS px,
    /// where `apply(value)` builds the full candidate [`Case`] with that one scalar
    /// substituted (everything else held at its current value in the working case).
    ///
    /// Stage 1 coarsens `current` toward `target` by snapping to ever-finer
    /// powers-of-two grid boundaries, restarting from the coarsest boundary again after
    /// every accepted snap (since a snap changes how much distance is left to cover).
    /// Stage 2 binary-searches the grid between the result (known-preserving) and
    /// `target` (known non-preserving — an earlier rule this same pass already tried it
    /// exactly and failed), stepping a probe back toward the preserving side whenever it
    /// lands on an invalid grid point (e.g. a font size that trips the `FreeType` hack).
    fn ladder<F>(
        &mut self,
        current: f32,
        target: f32,
        steps_per_px: f32,
        mut apply: F,
    ) -> Result<f32, MinimiseError>
    where
        F: FnMut(f32) -> Case,
    {
        if current == target {
            return Ok(current);
        }

        let steps_per_px = f64::from(steps_per_px);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "every sampled scalar's grid-step count stays far inside i64's range"
        )]
        let to_steps = |value: f32| -> i64 { (f64::from(value) * steps_per_px).round() as i64 };
        let from_steps = |steps: i64| -> f32 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "every sampled scalar's value stays far inside f32's range"
            )]
            let value = (steps as f64 / steps_per_px) as f32;
            value
        };

        let mut cur_steps = to_steps(current);
        let target_steps = to_steps(target);

        // Stage 1: coarsen toward `target_steps`, restarting from the top on every
        // acceptance.
        loop {
            let distance = target_steps.abs_diff(cur_steps);
            if distance == 0 {
                break;
            }
            #[expect(
                clippy::cast_possible_wrap,
                reason = "grid-step distances stay far inside i64's positive range"
            )]
            let mut k = largest_pow2_leq(distance) as i64;
            let mut progressed = false;
            while k >= 2 {
                let candidate_steps = snap_toward(cur_steps, target_steps, k);
                if candidate_steps != cur_steps {
                    let candidate = apply(from_steps(candidate_steps));
                    if is_valid_candidate(&candidate) && self.preserves(&candidate)? {
                        cur_steps = candidate_steps;
                        progressed = true;
                        break;
                    }
                }
                k /= 2;
            }
            if !progressed {
                break;
            }
        }

        // Stage 2: binary search between `cur_steps` (preserving) and `target_steps`
        // (known non-preserving).
        let mut lo = cur_steps;
        let mut hi = target_steps;
        while hi.abs_diff(lo) > 1 {
            let original_mid = (lo + hi).div_euclid(2);
            let mut mid = original_mid;
            let step: i64 = if mid < lo { 1 } else { -1 };
            let mut candidate = apply(from_steps(mid));
            while !is_valid_candidate(&candidate) && mid != lo {
                mid += step;
                candidate = apply(from_steps(mid));
            }
            if mid == lo {
                // Every grid point strictly between the original midpoint and `lo` was
                // invalid: treat the whole probe as a rejection at the point we were
                // actually asked about.
                hi = original_mid;
            } else if self.preserves(&candidate)? {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        Ok(from_steps(lo))
    }

    /// Rule (f): per run (ascending), per character position (ascending), tries
    /// replacing that character with the lowest [`alphabet`] entry that stays valid and
    /// signature-preserving — the global minimum for that position, not just a local
    /// decrement. Spaces have no alphabet index and are left to [`Self::ddmin_chars`].
    fn char_scan(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        let alphabet = alphabet();
        for run_index in 0..working.runs.len() {
            let mut chars: Vec<char> = working.runs[run_index].text.chars().collect();
            for position in 0..chars.len() {
                let ch = chars[position];
                if ch == ' ' {
                    continue;
                }
                // Scanning above the current character is provably wasted: it already
                // preserves the signature and is, by construction, not lower.
                let ceiling = alphabet.binary_search(&ch).unwrap_or(alphabet.len());
                let scan_limit = ceiling.min(CHAR_SCAN_CAP);
                for &candidate_char in &alphabet[..scan_limit] {
                    let mut candidate_chars = chars.clone();
                    candidate_chars[position] = candidate_char;
                    let mut candidate = working.clone();
                    candidate.runs[run_index].text = candidate_chars.iter().collect();
                    if let Some(accepted) = self.try_candidate(candidate)? {
                        *working = accepted;
                        chars[position] = candidate_char;
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Returns the largest power of two that is `<= n`. `n` must be at least `1`.
fn largest_pow2_leq(n: u64) -> u64 {
    debug_assert!(n >= 1, "largest_pow2_leq is only ever called with n >= 1");
    1_u64 << n.ilog2()
}

/// Returns the multiple of `k` closest to `cur`, strictly on the side of `cur` toward
/// `target`, clamped so it never passes `target`. Requires `cur != target` and `k >= 1`.
fn snap_toward(cur: i64, target: i64, k: i64) -> i64 {
    debug_assert!(
        cur != target,
        "snap_toward is only ever called with cur != target"
    );
    let floor_multiple = cur.div_euclid(k) * k;
    let candidate = if target > cur {
        floor_multiple + k
    } else if floor_multiple < cur {
        floor_multiple
    } else {
        floor_multiple - k
    };
    if target > cur {
        candidate.min(target)
    } else {
        candidate.max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::GlyphDiff;
    use crate::generate::Run;
    use crate::glyph_output::{PositionedGlyph, Style};

    /// A minimal, valid two-character `Case`: short enough that tests run fast, but
    /// with enough characters that ddmin/char-scan rules have something to do.
    fn case(text: &str, width: f32, font_size: f32) -> Case {
        Case {
            seed: 0,
            runs: vec![Run {
                text: text.to_string(),
                font_size,
                letter_spacing: 0.0,
                word_spacing: 0.0,
                line_height: font_size,
            }],
            width,
        }
    }

    /// A `Mismatch::GlyphCount` for use as a stand-in failure.
    fn glyph_count_mismatch() -> Mismatch {
        Mismatch::GlyphCount {
            parley: 1,
            chrome: 2,
        }
    }

    /// A `Mismatch::Glyphs` with x-drift, for tests that need a *different* signature
    /// than [`glyph_count_mismatch`].
    fn glyphs_mismatch() -> Mismatch {
        let style = Style {
            postscript_name: "Roboto-Regular".to_string(),
            font_size: 16.0,
        };
        let glyph = PositionedGlyph {
            id: 1,
            x: 0.0,
            y: 0.0,
            style: 0,
        };
        Mismatch::Glyphs {
            diffs: vec![GlyphDiff {
                index: 0,
                parley: glyph,
                chrome: glyph,
                parley_style: style.clone(),
                chrome_style: style,
                dx: 5.0,
                dy: 0.0,
                x_tolerance: 0.01,
                y_tolerance: 0.01,
            }],
            total: 1,
            same_multiset: false,
        }
    }

    /// Builds an [`Oracle`] closure from a predicate on the case's text: fails (with
    /// [`glyph_count_mismatch`]) whenever `pred` returns `true`, else passes.
    fn fails_when(
        mut pred: impl FnMut(&Case) -> bool + 'static,
    ) -> impl FnMut(&Case) -> Result<Result<(), Mismatch>, OracleFailure> {
        move |case: &Case| {
            if pred(case) {
                Ok(Err(glyph_count_mismatch()))
            } else {
                Ok(Ok(()))
            }
        }
    }

    #[test]
    fn baseline_passing_case_is_rejected() {
        let original = case("ab", MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = fails_when(|_| false);
        let result = minimise(&original, &mut oracle);
        assert!(
            matches!(result, Err(MinimiseError::BaselinePasses)),
            "a case the oracle reports as passing must return BaselinePasses"
        );
    }

    #[test]
    fn fatal_baseline_oracle_failure_aborts() {
        let original = case("ab", MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = |_: &Case| {
            Err::<Result<(), Mismatch>, _>(OracleFailure {
                message: "boom".to_string(),
                fatal: true,
            })
        };
        let result = minimise(&original, &mut oracle);
        assert!(
            matches!(result, Err(MinimiseError::Oracle(_))),
            "a fatal oracle failure on the baseline must abort with MinimiseError::Oracle"
        );
    }

    #[test]
    fn ddmin_reduces_to_a_single_essential_character() {
        let letters = alphabet();
        // Pick a character that is not the essential one, to build filler text with.
        let filler = *letters
            .iter()
            .find(|&&c| c != 'e')
            .expect("alphabet has more than one character");
        let text: String = std::iter::repeat_n(filler, 40).collect();
        let original = case(&text, MAX_CASE_WIDTH_PX, 16.0);

        let mut oracle = fails_when(|c: &Case| c.runs[0].text.contains('e'));
        // 'e' has to actually be present for this predicate to ever fail — insert one.
        let mut original = original;
        original.runs[0].text.push('e');

        let outcome = minimise(&original, &mut oracle).expect("original fails, so this succeeds");
        assert_eq!(
            outcome.case.runs[0].text, "e",
            "ddmin should shrink the text down to just the essential character"
        );
        assert!(
            outcome.case.runs.len() == 1 && !outcome.case.runs[0].text.is_empty(),
            "the minimised case must still have at least one non-empty run"
        );
    }

    #[test]
    fn exact_targets_are_reached_when_the_predicate_ignores_them() {
        // A predicate that only cares about the text, so width/font_size/spacings are
        // all free to snap to their exact targets.
        let original = case("ee", 500.0, 20.0);
        let mut oracle = fails_when(|c: &Case| c.runs[0].text.contains('e'));

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert_eq!(
            outcome.case.width, MAX_CASE_WIDTH_PX,
            "width must reach its exact target"
        );
        assert_eq!(
            outcome.case.runs[0].font_size,
            target_font_size(),
            "font_size must reach its exact target"
        );
        assert_eq!(outcome.case.runs[0].letter_spacing, 0.0);
        assert_eq!(outcome.case.runs[0].word_spacing, 0.0);
    }

    #[test]
    fn ladder_shrinks_width_toward_but_not_past_a_threshold() {
        let original = case("ee", 200.0, 16.0);
        let mut oracle = fails_when(|c: &Case| c.width < 700.0);

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert!(
            oracle_predicate_width_fails(outcome.case.width),
            "the minimised width must still satisfy the failing predicate (< 700)"
        );
        assert!(
            outcome.case.width > original.width,
            "the ladder must move width toward its target (up, here), not leave it at \
             the original value"
        );
    }

    /// Mirrors the `width < 700.0` predicate from
    /// [`ladder_shrinks_width_toward_but_not_past_a_threshold`], kept as a standalone
    /// function so the assertion above reads as "still fails", not a duplicated literal.
    fn oracle_predicate_width_fails(width: f32) -> bool {
        width < 700.0
    }

    #[test]
    fn every_accepted_candidate_is_a_valid_case() {
        let original = case("eeeeeeeeee", 500.0, 20.0);
        let mut inner = fails_when(|c: &Case| c.runs[0].text.len() > 1);
        let mut oracle = move |c: &Case| {
            assert!(
                is_valid_candidate(c),
                "minimise must never show the oracle an invalid candidate: {c:?}"
            );
            inner(c)
        };

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert!(
            is_valid_candidate(&outcome.case),
            "the final minimised case must itself be valid"
        );
    }

    #[test]
    fn a_different_failure_signature_does_not_count_as_preserving() {
        // The baseline fails with GlyphCount; any candidate with shorter text instead
        // fails with a Glyphs mismatch, which must not be accepted as preserving.
        let original = case("eeee", MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = move |c: &Case| {
            if c.runs[0].text.len() < 4 {
                Ok(Err(glyphs_mismatch()))
            } else {
                Ok(Err(glyph_count_mismatch()))
            }
        };

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert_eq!(
            outcome.case.runs[0].text.len(),
            4,
            "a shrink that flips the failure signature must be rejected, leaving the \
             text unshrunk"
        );
        assert_eq!(outcome.signature, FailureSignature::GlyphCount);
    }

    #[test]
    fn non_fatal_oracle_errors_are_skipped_and_recorded() {
        let original = case("ee", 500.0, 16.0);
        let mut oracle = move |c: &Case| {
            if c.width == MAX_CASE_WIDTH_PX {
                Err(OracleFailure {
                    message: "transient\nerror".to_string(),
                    fatal: false,
                })
            } else if c.runs[0].text.contains('e') {
                Ok(Err(glyph_count_mismatch()))
            } else {
                Ok(Ok(()))
            }
        };

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert!(
            !outcome.skips.is_empty(),
            "a non-fatal oracle error on a candidate must be recorded as a skip"
        );
        for skip in &outcome.skips {
            assert!(
                !skip.contains('\n'),
                "a skip message must be a single line, got {skip:?}"
            );
        }
    }

    #[test]
    fn a_fatal_oracle_error_on_a_candidate_aborts() {
        let original = case("ee", 500.0, 16.0);
        let mut oracle = move |c: &Case| {
            if c.width == MAX_CASE_WIDTH_PX {
                Err(OracleFailure {
                    message: "fatal".to_string(),
                    fatal: true,
                })
            } else {
                Ok(Err(glyph_count_mismatch()))
            }
        };

        let result = minimise(&original, &mut oracle);
        assert!(
            matches!(result, Err(MinimiseError::Oracle(_))),
            "a fatal oracle failure on a candidate must abort minimisation"
        );
    }

    #[test]
    fn minimisation_is_deterministic() {
        let original = case("elephant seed text here", 500.0, 20.0);
        let predicate = |c: &Case| c.runs[0].text.contains('e');

        let mut oracle_a = fails_when(predicate);
        let outcome_a = minimise(&original, &mut oracle_a).expect("original fails");

        let mut oracle_b = fails_when(predicate);
        let outcome_b = minimise(&original, &mut oracle_b).expect("original fails");

        assert_eq!(
            outcome_a.case, outcome_b.case,
            "the minimised case must be deterministic"
        );
        assert_eq!(outcome_a.signature, outcome_b.signature);
        assert_eq!(outcome_a.passes, outcome_b.passes);
        assert_eq!(outcome_a.reached_fixed_point, outcome_b.reached_fixed_point);
        assert_eq!(outcome_a.candidates_tried, outcome_b.candidates_tried);
        assert_eq!(outcome_a.skips, outcome_b.skips);
    }

    #[test]
    fn char_scan_finds_the_lowest_satisfying_character_and_respects_the_cap() {
        let letters = alphabet();
        // A character comfortably above the very start of the alphabet, so there is
        // room for the scan to find something lower than it.
        let original_char = letters[letters.len() / 2];
        let threshold_index = letters.len() / 4;
        let threshold_char = letters[threshold_index];

        let original = case(&original_char.to_string(), MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = move |c: &Case| {
            let ch = c.runs[0]
                .text
                .chars()
                .next()
                .expect("case always has non-empty text");
            if ch >= threshold_char {
                Ok(Err(glyph_count_mismatch()))
            } else {
                Ok(Ok(()))
            }
        };

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert_eq!(
            outcome.case.runs[0].text,
            threshold_char.to_string(),
            "the char scan must canonicalise to the lowest alphabet character that still \
             satisfies the predicate"
        );

        let original_index = letters
            .binary_search(&original_char)
            .expect("original_char came from alphabet()");
        let bound = original_index.min(CHAR_SCAN_CAP);
        let bound_u64: u64 = bound
            .try_into()
            .expect("bound (an alphabet index) fits in u64");
        assert!(
            bound_u64 >= outcome.candidates_tried.saturating_sub(1),
            "the per-position scan must not exceed min(alphabet index, CHAR_SCAN_CAP) \
             candidates: bound {bound}, tried {}",
            outcome.candidates_tried
        );
    }

    #[test]
    fn pass_count_is_capped() {
        // A predicate that keeps the case failing regardless of width, but where the
        // width ladder can always find one more grid step to try moving on any given
        // pass (since the predicate never lets width settle at an exact target), so the
        // fixed-point loop is expected to run out the clock rather than converge.
        let original = case("ee", MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = fails_when(|c: &Case| c.width > 1.0);

        let outcome = minimise(&original, &mut oracle).expect("original fails");
        assert!(
            outcome.passes <= MAX_PASSES,
            "passes must never exceed MAX_PASSES, got {}",
            outcome.passes
        );
    }

    #[test]
    fn largest_pow2_leq_matches_expected_values() {
        assert_eq!(
            largest_pow2_leq(1),
            1,
            "1's largest power of two <= itself is 1"
        );
        assert_eq!(
            largest_pow2_leq(5),
            4,
            "5's largest power of two <= itself is 4"
        );
        assert_eq!(largest_pow2_leq(8), 8, "8 is itself a power of two");
    }

    #[test]
    fn snap_toward_moves_toward_target_without_passing_it() {
        assert_eq!(
            snap_toward(10, 100, 8),
            16,
            "snap up to the next multiple of 8"
        );
        assert_eq!(
            snap_toward(10, 0, 8),
            8,
            "snap down to the previous multiple of 8"
        );
        assert_eq!(
            snap_toward(10, 12, 8),
            12,
            "snapping must clamp at target rather than overshoot it"
        );
    }
}
