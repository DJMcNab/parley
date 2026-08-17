// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;

use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, Script};

use crate::chromium_quantization::{LAYOUT_UNIT_STEPS_PER_PX, SPACING_GRID_STEPS_PER_PX};
use crate::compare::Mismatch;
use crate::generate::{
    Case, FONT_SIZE_STEP, MAX_CASE_WIDTH_PX, MAX_FONT_SIZE, MIN_FONT_SIZE, alphabet,
    valid_font_size,
};
use crate::signature::FailureSignature;

const FONT_SIZE_STEPS_PER_PX: f32 = 1.0 / FONT_SIZE_STEP;

// A safety bound for passes whose reductions can unlock one another.
const MAX_PASSES: u32 = 5;

// A safety bound for exhaustive probing within one Unicode group. This is applied to
// the original character's group, not to the alphabet prefix, so scripts added by
// future fonts still get representatives and same-group probes.
const CHAR_GROUP_SCAN_CAP: usize = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CharGroup {
    script: Script,
    category: GeneralCategory,
}

fn char_group(ch: char) -> CharGroup {
    CharGroup {
        script: CodePointMapData::<Script>::new().get(ch),
        category: CodePointMapData::<GeneralCategory>::new().get(ch),
    }
}

/// Returns the ordered character probes for `ch`.
///
/// Characters sharing a Unicode script and general category with `ch` are all worth
/// trying: for example, changing one Latin lowercase letter can expose a glyph-specific
/// failure. Other groups get only their lowest-codepoint representative. If `a` does
/// not reproduce a failure in another group, scanning through `b` to `z` is unlikely
/// to justify 25 more Chrome captures. The grouping comes from Unicode properties, not
/// the currently bundled font, so additional fonts and scripts need no new table.
fn char_probes(ch: char) -> Vec<char> {
    let alphabet = alphabet();
    let ceiling = alphabet.binary_search(&ch).unwrap_or(alphabet.len());
    let own_group = char_group(ch);
    let mut own_group_count = 0;
    let mut represented_groups = Vec::new();
    let mut probes = Vec::new();

    for &candidate in &alphabet[..ceiling] {
        let group = char_group(candidate);
        if group == own_group {
            if own_group_count < CHAR_GROUP_SCAN_CAP {
                probes.push(candidate);
                own_group_count += 1;
            }
        } else if !represented_groups.contains(&group) {
            probes.push(candidate);
            represented_groups.push(group);
        }
    }
    probes
}

/// An error returned while evaluating a candidate.
#[derive(Clone, Debug)]
pub struct OracleFailure {
    /// Human-readable error.
    pub message: String,
    /// Whether minimisation must stop.
    pub fatal: bool,
}

/// Evaluates whether a case reproduces a mismatch.
pub trait Oracle {
    /// Returns the comparison result, or an evaluation error.
    fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure>;

    /// Optional cumulative counters used to attribute expensive oracle work to phases.
    fn activity(&self) -> Option<OracleActivity> {
        None
    }
}

impl<F> Oracle for F
where
    F: FnMut(&Case) -> Result<Result<(), Mismatch>, OracleFailure>,
{
    fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure> {
        self(case)
    }
}

/// Cumulative work performed by an oracle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OracleActivity {
    /// Uncached external evaluations, such as Chrome captures.
    pub captures: u64,
    /// Evaluations served from a cache.
    pub cache_hits: u64,
}

/// Work attributed to one minimisation phase across all passes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinimisePhaseStats {
    /// Candidates evaluated by this phase.
    pub candidates: u64,
    /// Uncached oracle captures performed by this phase.
    pub captures: u64,
    /// Oracle cache hits in this phase.
    pub cache_hits: u64,
}

/// Per-phase minimisation work counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinimiseStats {
    /// The required initial evaluation of the input case.
    pub baseline: MinimisePhaseStats,
    /// Exact container-width probes.
    pub width_exact: MinimisePhaseStats,
    /// Exact font-size and spacing probes.
    pub scalar_exacts: MinimisePhaseStats,
    /// Styled-run removal probes.
    pub remove_runs: MinimisePhaseStats,
    /// Delta-debugging character removal probes.
    pub ddmin_chars: MinimisePhaseStats,
    /// Scalar ladder and binary-search probes.
    pub scalar_ladder: MinimisePhaseStats,
    /// Character canonicalisation probes.
    pub char_scan: MinimisePhaseStats,
}

/// An error preventing minimisation.
#[derive(Debug)]
pub enum MinimiseError {
    /// The original case does not fail.
    BaselinePasses,
    /// The oracle could not evaluate a required case.
    Oracle(OracleFailure),
}

/// The result of minimising a failing case.
#[derive(Debug)]
pub struct MinimiseOutcome {
    /// Reduced case.
    pub case: Case,
    /// Failure category preserved during reduction.
    pub signature: FailureSignature,
    /// Mismatch from the last accepted candidate.
    pub mismatch: Mismatch,
    /// Number of reduction passes run.
    pub passes: u32,
    /// Whether a pass completed without changing the case.
    pub reached_fixed_point: bool,
    /// Number of candidates evaluated.
    pub candidates_tried: u64,
    /// Non-fatal oracle errors encountered.
    pub skips: Vec<String>,
    /// Work attributed to each reduction phase.
    pub stats: MinimiseStats,
}

/// Reduces `case` while preserving its initial failure category.
pub fn minimise(case: &Case, oracle: &mut dyn Oracle) -> Result<MinimiseOutcome, MinimiseError> {
    let baseline_activity = oracle.activity();
    let baseline_mismatch = match oracle.evaluate(case).map_err(MinimiseError::Oracle)? {
        Ok(()) => return Err(MinimiseError::BaselinePasses),
        Err(mismatch) => mismatch,
    };
    let mut stats = MinimiseStats {
        baseline: phase_stats(1, baseline_activity, oracle.activity()),
        ..MinimiseStats::default()
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

        let start = minimiser.phase_start();
        minimiser.width_exact(&mut working)?;
        minimiser.finish_phase(&mut stats.width_exact, start);

        let start = minimiser.phase_start();
        minimiser.scalar_exacts(&mut working, font_size_target)?;
        minimiser.finish_phase(&mut stats.scalar_exacts, start);

        let start = minimiser.phase_start();
        minimiser.remove_runs(&mut working)?;
        minimiser.finish_phase(&mut stats.remove_runs, start);

        let start = minimiser.phase_start();
        minimiser.ddmin_chars(&mut working)?;
        minimiser.finish_phase(&mut stats.ddmin_chars, start);

        let start = minimiser.phase_start();
        minimiser.scalar_ladder(&mut working, font_size_target)?;
        minimiser.finish_phase(&mut stats.scalar_ladder, start);

        let start = minimiser.phase_start();
        minimiser.char_scan(&mut working)?;
        minimiser.finish_phase(&mut stats.char_scan, start);

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
        stats,
    })
}

fn phase_stats(
    candidates: u64,
    before: Option<OracleActivity>,
    after: Option<OracleActivity>,
) -> MinimisePhaseStats {
    let activity = match (before, after) {
        (Some(before), Some(after)) => OracleActivity {
            captures: after.captures.saturating_sub(before.captures),
            cache_hits: after.cache_hits.saturating_sub(before.cache_hits),
        },
        _ => OracleActivity::default(),
    };
    MinimisePhaseStats {
        candidates,
        captures: activity.captures,
        cache_hits: activity.cache_hits,
    }
}

#[must_use]
/// Returns a stable key for a case's contents, excluding its seed.
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

fn terse(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Minimiser<'o> {
    oracle: &'o mut dyn Oracle,
    baseline: FailureSignature,
    best_mismatch: Mismatch,
    skips: Vec<String>,
    candidates_tried: u64,
}

struct PhaseStart {
    candidates: u64,
    activity: Option<OracleActivity>,
}

impl Minimiser<'_> {
    fn phase_start(&self) -> PhaseStart {
        PhaseStart {
            candidates: self.candidates_tried,
            activity: self.oracle.activity(),
        }
    }

    fn finish_phase(&self, stats: &mut MinimisePhaseStats, start: PhaseStart) {
        let delta = phase_stats(
            self.candidates_tried.saturating_sub(start.candidates),
            start.activity,
            self.oracle.activity(),
        );
        stats.candidates += delta.candidates;
        stats.captures += delta.captures;
        stats.cache_hits += delta.cache_hits;
    }

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

    fn width_exact(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        // A wider container is simpler here because it removes line breaks.
        if working.width != MAX_CASE_WIDTH_PX {
            let mut candidate = working.clone();
            candidate.width = MAX_CASE_WIDTH_PX;
            if let Some(accepted) = self.try_candidate(candidate)? {
                *working = accepted;
            }
        }
        Ok(())
    }

    fn scalar_exacts(
        &mut self,
        working: &mut Case,
        font_size_target: f32,
    ) -> Result<(), MinimiseError> {
        // Line height has no neutral target comparable to zero spacing or a 16px font.
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

    fn remove_runs(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        let mut index = 0;
        while index < working.runs.len() {
            if working.runs.len() > 1 {
                let mut candidate = working.clone();
                candidate.runs.remove(index);
                if let Some(accepted) = self.try_candidate(candidate)? {
                    *working = accepted;
                    // The next run shifted into this index.
                    continue;
                }
            }
            index += 1;
        }
        Ok(())
    }

    fn ddmin_chars(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        for run_index in 0..working.runs.len() {
            self.ddmin_run(working, run_index)?;
        }
        Ok(())
    }

    fn ddmin_run(&mut self, working: &mut Case, run_index: usize) -> Result<(), MinimiseError> {
        // Work in characters because the generated alphabet is not ASCII-only.
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

        // Probe progressively finer grid boundaries, restarting after each success.
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

        let mut lo = cur_steps;
        let mut hi = target_steps;
        // `lo` preserves the failure; the exact target in `hi` was rejected earlier.
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
                hi = original_mid;
            } else if self.preserves(&candidate)? {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        Ok(from_steps(lo))
    }

    fn char_scan(&mut self, working: &mut Case) -> Result<(), MinimiseError> {
        for run_index in 0..working.runs.len() {
            let mut chars: Vec<char> = working.runs[run_index].text.chars().collect();
            for position in 0..chars.len() {
                let ch = chars[position];
                if ch == ' ' {
                    continue;
                }
                for candidate_char in char_probes(ch) {
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

fn largest_pow2_leq(n: u64) -> u64 {
    debug_assert!(n >= 1, "largest_pow2_leq is only ever called with n >= 1");
    1_u64 << n.ilog2()
}

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

    fn glyph_count_mismatch() -> Mismatch {
        Mismatch::GlyphCount {
            parley: 1,
            chrome: 2,
        }
    }

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

    struct CountingOracle {
        captures: u64,
    }

    impl Oracle for CountingOracle {
        fn evaluate(&mut self, case: &Case) -> Result<Result<(), Mismatch>, OracleFailure> {
            self.captures += 1;
            if case.runs[0].text.contains('e') {
                Ok(Err(glyph_count_mismatch()))
            } else {
                Ok(Ok(()))
            }
        }

        fn activity(&self) -> Option<OracleActivity> {
            Some(OracleActivity {
                captures: self.captures,
                cache_hits: 0,
            })
        }
    }

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
        let filler = *letters
            .iter()
            .find(|&&c| c != 'e')
            .expect("alphabet has more than one character");
        let text: String = std::iter::repeat_n(filler, 40).collect();
        let original = case(&text, MAX_CASE_WIDTH_PX, 16.0);

        let mut oracle = fails_when(|c: &Case| c.runs[0].text.contains('e'));
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
    fn phase_stats_account_for_every_evaluation() {
        let original = case("elephant", 500.0, 20.0);
        let mut oracle = CountingOracle { captures: 0 };
        let outcome = minimise(&original, &mut oracle).expect("original fails");
        let phases = [
            outcome.stats.baseline,
            outcome.stats.width_exact,
            outcome.stats.scalar_exacts,
            outcome.stats.remove_runs,
            outcome.stats.ddmin_chars,
            outcome.stats.scalar_ladder,
            outcome.stats.char_scan,
        ];
        assert_eq!(outcome.stats.baseline.candidates, 1);
        assert_eq!(
            phases.iter().map(|phase| phase.candidates).sum::<u64>(),
            outcome.candidates_tried + 1,
            "the baseline plus reduction phases must account for every candidate"
        );
        assert_eq!(
            phases.iter().map(|phase| phase.captures).sum::<u64>(),
            oracle.captures,
            "the phase capture deltas must equal the oracle's cumulative counter"
        );
        assert_eq!(phases.iter().map(|phase| phase.cache_hits).sum::<u64>(), 0);
    }

    #[test]
    fn char_scan_finds_the_lowest_satisfying_character_in_the_original_group() {
        let letters = alphabet();
        let original_char = 'z';
        let original_group = char_group(original_char);
        let same_group: Vec<char> = letters
            .iter()
            .copied()
            .take_while(|&ch| ch < original_char)
            .filter(|&ch| char_group(ch) == original_group)
            .collect();
        let threshold_char = same_group[same_group.len() / 2];

        let original = case(&original_char.to_string(), MAX_CASE_WIDTH_PX, 16.0);
        let mut oracle = move |c: &Case| {
            let ch = c.runs[0]
                .text
                .chars()
                .next()
                .expect("case always has non-empty text");
            if char_group(ch) == original_group && ch >= threshold_char {
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
    }

    #[test]
    fn char_probes_scan_only_the_original_group_fully() {
        let original = 'z';
        let original_group = char_group(original);
        let probes = char_probes(original);
        let candidates = &alphabet()[..alphabet()
            .binary_search(&original)
            .expect("z is in the bundled font")];

        let expected_same_group: Vec<char> = candidates
            .iter()
            .copied()
            .filter(|&ch| char_group(ch) == original_group)
            .collect();
        let actual_same_group: Vec<char> = probes
            .iter()
            .copied()
            .filter(|&ch| char_group(ch) == original_group)
            .collect();
        assert_eq!(actual_same_group, expected_same_group);

        for &probe in &probes {
            let group = char_group(probe);
            if group != original_group {
                assert_eq!(
                    probes
                        .iter()
                        .filter(|&&candidate| char_group(candidate) == group)
                        .count(),
                    1,
                    "a foreign script/category group must contribute only its representative"
                );
            }
        }
    }

    #[test]
    fn pass_count_is_capped() {
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
