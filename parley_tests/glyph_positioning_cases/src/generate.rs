// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Seeded random generation of [`Case`]s.
//!
//! See "The `Case` grammar" and "Generation" in
//! `doc/glyph-positioning-chrome-parity-phase1.md`.

use std::sync::OnceLock;

use rand::{RngExt, SeedableRng, seq::IndexedRandom};
use rand_chacha::ChaCha8Rng;
use read_fonts::{FontRef, TableProvider};

use crate::FONTS;
use crate::chromium_quantization::{SPACING_GRID_STEPS_PER_PX, floor_to_layout_unit, is_on_grid};
use crate::freetype_hack::hack_would_fire;

/// A single seed-derived glyph-positioning test case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// Provenance only. Never used to regenerate the case at test time.
    pub seed: u64,
    /// The styled text runs making up the case, in order.
    pub runs: Vec<Run>,
    /// The container width, in CSS px. Always an exact multiple of 1/64.
    pub width: f32,
}

/// A single styled run of text within a [`Case`].
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The run's text.
    pub text: String,
    /// Font size, in CSS px, pre-truncation. See "font size" in the Phase 1 doc.
    pub font_size: f32,
    /// Extra spacing between letters, in CSS px. Always an exact multiple of 1/256.
    pub letter_spacing: f32,
    /// Extra spacing between words, in CSS px. Always an exact multiple of 1/256.
    pub word_spacing: f32,
    /// Line height, in CSS px — an **absolute length** (`harness_css::run_css` emits
    /// e.g. `line-height:20px`, never a unitless number or `normal`). Always an exact
    /// multiple of 1/64, `width`'s `LayoutUnit` grid (see [`floor_to_layout_unit`]).
    ///
    /// Sampled per-run, independently of every other run in the case (matching
    /// `font_size`/`letter_spacing`/`word_spacing`), so a case can exercise the line
    /// box being a union across runs whose line heights disagree, not just their
    /// ascents/descents.
    ///
    /// Absolute px was chosen over a unitless number specifically so the sampled value
    /// itself is the exact px length Blink lays out with, on the same grid `width`
    /// already uses — a unitless number's effective px value depends on `font_size`
    /// too, which would need a second grid-intersection argument (as `FONT_SIZE_STEP`
    /// required) to land safely. `normal` was ruled out for v1 for the opposite
    /// reason: it isn't a value this crate chooses, so it can't be grid-sampled at
    /// all; it also carries a first-available-font subtlety this crate doesn't need to
    /// worry about, since v1 only ever has one font ([`crate::FONTS`] has one entry).
    pub line_height: f32,
}

/// Inclusive bounds on the number of runs in a generated case.
/// **Temporarily pinned to 1 run per case.** With more than one run, a run boundary's
/// position is `SnappedWidth()`-ceil-rounded to `LayoutUnit`'s 1/64 px grid on the Chrome
/// side (see bring-up step B13 in the Phase 4 doc — confirmed from
/// `third_party/blink/renderer/platform/fonts/shaping/shape_result.h:170`) and not
/// modelled on the Parley side, so every case with ≥2 runs carries an unmodelled ~1/64 px
/// step at each boundary. A single run has no run boundary to step at, so this defers
/// modelling that rounding rather than working around it with a fudge factor. Restore
/// `MAX_RUNS` to 4 once that rounding is modelled in
/// `parley_glyph_positioning_extract`.
///
/// This does not defer B13 as a whole: bring-up found it can still appear at a *line*
/// boundary within a single run (e.g. a hanging trailing space per B9), since Blink
/// positions that space as a fragment following the line's main text fragment the same
/// way it positions a second run. Restricting run count only removes the *cross-run*
/// occurrence.
const MIN_RUNS: usize = 1;
const MAX_RUNS: usize = 1;

/// Inclusive bounds on the total text length (in characters) of a generated case.
const MIN_TOTAL_LEN: usize = 30;
const MAX_TOTAL_LEN: usize = 120;

/// The probability of inserting a space at any position where one is legal (i.e. not
/// the first character of a run, and not immediately after another space).
const SPACE_PROBABILITY: f64 = 0.18;

/// Visible at `pub(crate)` rather than private so the minimiser (`src/minimise.rs`) can
/// compute its own target font size and grid-check candidates without duplicating this
/// grammar constant.
pub(crate) const MIN_FONT_SIZE: f32 = 10.0;
/// See [`MIN_FONT_SIZE`].
pub(crate) const MAX_FONT_SIZE: f32 = 30.0;

/// The grid font sizes are sampled on: **1/4 CSS px**.
///
/// This is where Blink's two size grids intersect. Blink truncates a font size to 1/100
/// px for its font cache key, *and* quantizes it to `LayoutUnit`'s 1/64 px grid; a
/// multiple of 1/4 is exactly representable on both, so neither quantization moves it and
/// Parley and Blink shape at the same size.
///
/// **Phase 4 correction.** Phase 1 sampled a 1/100 grid plus a sub-hundredth offset,
/// deliberately exercising the 1/100 truncation. That left the 1/64 size quantization
/// unmodelled, which the first fuzz run measured as a *relative* x error of up to
/// 1.3e-3 — around a hundred times the 16.16 accumulation the plan had budgeted for, and
/// growing with x rather than with glyph index. Sampling on the intersection removes the
/// confound outright rather than modelling it. See the Phase 4 doc's findings.
pub(crate) const FONT_SIZE_STEP: f32 = 0.25;
#[expect(
    clippy::cast_possible_truncation,
    reason = "We know this doesn't overflow."
)]
const FONT_SIZE_STEPS: u32 = ((MAX_FONT_SIZE - MIN_FONT_SIZE) / FONT_SIZE_STEP) as u32;

/// Inclusive bounds on the container width as a multiplier of the largest run's font
/// size, matching `parley_linebreaking_cases`.
const MIN_EM_FACTOR: f32 = 7.0;
const MAX_EM_FACTOR: f32 = 32.0;

/// The cap on a case's width, chosen so no glyph `x` exceeds roughly this value. See
/// "Shape" in the Phase 1 doc.
///
/// Public so the minimiser (`src/minimise.rs`) can both target this exact value (its
/// width-exact rule) and validate that a shrunk candidate's width never exceeds it.
pub const MAX_CASE_WIDTH_PX: f32 = 1000.0;

/// Inclusive bounds, in CSS px, for sampled letter spacing.
///
/// **Temporarily pinned to zero — recommended non-zero range to restore is `(-0.5,
/// 2.5)`, Phase 1's original bound.** Non-zero letter spacing exposed a real Parley bug,
/// unrelated to this harness: a base character requiring font-level decomposition (no
/// precomposed glyph, so its own accent is synthesized under the base's `HarfBuzz`
/// cluster id), followed by a *separate* explicit combining-mark codepoint (its own
/// distinct cluster id), is split into two Parley "shaped clusters"
/// (`parley_engine/src/shape/shaped_text.rs:435-525`, which clusters purely on
/// `glyph_info.cluster`) instead of one. `LayoutData::finish`
/// (`parley/src/layout/data.rs:341-368`) then adds spacing once per cluster, so that one
/// grapheme gets it twice — confirmed against real Chrome captures as an offset exactly
/// equal to the run's `letter_spacing`, appearing only at exactly these characters. Fix
/// the cluster-merging bug before restoring a non-zero range; re-widening this without
/// that fix just reintroduces cases the corpus can't pass.
const LETTER_SPACING_RANGE_PX: (f32, f32) = (0.0, 0.0);
/// Inclusive bounds, in CSS px, for sampled word spacing.
///
/// **Temporarily pinned to zero — recommended non-zero range to restore is `(-1.0,
/// 4.0)`, Phase 1's original bound.** Same bug as [`LETTER_SPACING_RANGE_PX`]: word
/// spacing is added once per over-split cluster too, whenever the mark-splitting
/// condition happens to land next to a whitespace cluster. Confirmed against real Chrome
/// captures as an offset exactly equal to the run's `word_spacing`. Restore alongside
/// letter spacing, once the underlying cluster-merging bug is fixed — not before, for
/// the same reason.
const WORD_SPACING_RANGE_PX: (f32, f32) = (0.0, 0.0);

/// Inclusive bounds on a run's line height, expressed as a multiplier of that run's own
/// `font_size`. Only used to pick a plausible target at generation time — the CSS
/// emitted is always an absolute length (see [`Run::line_height`]), so this range
/// exists purely so generated cases don't sample implausible line heights (e.g. far
/// smaller than the glyphs they'd have to contain), not because the grammar itself is
/// relative.
const LINE_HEIGHT_FACTOR_RANGE: (f32, f32) = (0.8, 2.0);

impl Case {
    /// Generates the [`Case`] for a given seed.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let alphabet = alphabet();

        let num_runs = rng.random_range(MIN_RUNS..=MAX_RUNS);
        let total_len = rng.random_range(MIN_TOTAL_LEN..=MAX_TOTAL_LEN);
        let lengths = partition_length(&mut rng, total_len, num_runs);

        let mut prev_was_space = true;
        let runs: Vec<Run> = lengths
            .into_iter()
            .map(|len| {
                let text = generate_run_text(&mut rng, alphabet, len, &mut prev_was_space);
                let font_size = sample_font_size(&mut rng);
                let letter_spacing = sample_spacing(&mut rng, LETTER_SPACING_RANGE_PX);
                let word_spacing = sample_spacing(&mut rng, WORD_SPACING_RANGE_PX);
                let line_height = sample_line_height(&mut rng, font_size);
                Run {
                    text,
                    font_size,
                    letter_spacing,
                    word_spacing,
                    line_height,
                }
            })
            .collect();

        let max_font_size = runs
            .iter()
            .map(|run| run.font_size)
            .fold(f32::MIN, f32::max);
        let em_factor = rng.random_range(MIN_EM_FACTOR..=MAX_EM_FACTOR);
        let width = floor_to_layout_unit((max_font_size * em_factor).min(MAX_CASE_WIDTH_PX));

        Self { seed, runs, width }
    }
}

/// Splits `total_len` into `num_runs` positive-length pieces.
fn partition_length(rng: &mut ChaCha8Rng, total_len: usize, num_runs: usize) -> Vec<usize> {
    let mut lengths = vec![1_usize; num_runs];
    for _ in 0..(total_len - num_runs) {
        let index = rng.random_range(0..num_runs);
        lengths[index] += 1;
    }
    lengths
}

/// Generates `len` characters of run text sampled from `alphabet`, subject to the
/// space-insertion rules in "Spaces" in the Phase 1 doc: no run may begin with a
/// space, and no two U+0020 may be adjacent (including across a run boundary, tracked
/// via `prev_was_space`).
fn generate_run_text(
    rng: &mut ChaCha8Rng,
    alphabet: &[char],
    len: usize,
    prev_was_space: &mut bool,
) -> String {
    let mut text = String::with_capacity(len);
    for position in 0..len {
        let at_run_start = position == 0;
        let can_place_space = !*prev_was_space && !at_run_start;
        let ch = if can_place_space && rng.random_bool(SPACE_PROBABILITY) {
            ' '
        } else {
            *alphabet.choose(rng).expect("alphabet is non-empty")
        };
        text.push(ch);
        *prev_was_space = ch == ' ';
    }
    text
}

/// Samples a font size in `[MIN_FONT_SIZE, MAX_FONT_SIZE]`, on the [`FONT_SIZE_STEP`]
/// grid, rerolling from the same RNG stream whenever the `FreeType` ascent/descent hack
/// would fire for the bundled font at this size.
///
/// Sizes land exactly on the grid, with no sub-step offset: they are chosen so *neither*
/// of Blink's size quantizations moves them, which makes [`quantize_font_size`] a no-op
/// here by construction rather than something the corpus has to exercise.
fn sample_font_size(rng: &mut ChaCha8Rng) -> f32 {
    let (hhea_descender, units_per_em) = roboto_metrics();
    loop {
        let size = MIN_FONT_SIZE + rng.random_range(0..=FONT_SIZE_STEPS) as f32 * FONT_SIZE_STEP;
        if !hack_would_fire(hhea_descender, units_per_em, size) {
            return size;
        }
    }
}

/// Samples a line height in CSS px for a run whose font size is `font_size`: a
/// multiplier of `font_size` uniform in [`LINE_HEIGHT_FACTOR_RANGE`], floored to
/// `width`'s `LayoutUnit` 1/64 px grid via [`floor_to_layout_unit`] so it survives
/// Blink's fixed-point conversion losslessly.
fn sample_line_height(rng: &mut ChaCha8Rng, font_size: f32) -> f32 {
    let (min_factor, max_factor) = LINE_HEIGHT_FACTOR_RANGE;
    floor_to_layout_unit(font_size * rng.random_range(min_factor..=max_factor))
}

/// Samples a spacing value (letter or word spacing), in CSS px, uniformly on the
/// [`SPACING_GRID_STEPS_PER_PX`] grid within `[min_px, max_px]`.
fn sample_spacing(rng: &mut ChaCha8Rng, (min_px, max_px): (f32, f32)) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "spacing bounds are small constants, far inside i32's range"
    )]
    let min_steps = (min_px * SPACING_GRID_STEPS_PER_PX).round() as i32;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "spacing bounds are small constants, far inside i32's range"
    )]
    let max_steps = (max_px * SPACING_GRID_STEPS_PER_PX).round() as i32;
    rng.random_range(min_steps..=max_steps) as f32 / SPACING_GRID_STEPS_PER_PX
}

/// Returns `(hhea_descender, units_per_em)` for the bundled Roboto font ([`FONTS`]`[0]`).
fn roboto_metrics() -> (i16, u16) {
    static METRICS: OnceLock<(i16, u16)> = OnceLock::new();
    *METRICS.get_or_init(|| {
        let font = FontRef::new(FONTS[0].bytes).expect("bundled font parses");
        let hhea = font.hhea().expect("bundled font has hhea");
        let head = font.head().expect("bundled font has head");
        (hhea.descender().to_i16(), head.units_per_em())
    })
}

/// Returns the sampling alphabet for [`FONTS`]`[0]`: its `cmap` intersected with the
/// Basic Multilingual Plane, minus the `Cc`/`Zs`/`Cf`/`Co` hazard classes. See
/// "Alphabet" in the Phase 1 doc.
///
/// Sorted and deduplicated (see [`compute_alphabet`]), and public so the minimiser
/// (`src/minimise.rs`) can validate candidate text against it and binary-search it for
/// its per-character min-first scan, without recomputing or duplicating it.
#[must_use]
pub fn alphabet() -> &'static [char] {
    static ALPHABET: OnceLock<Vec<char>> = OnceLock::new();
    ALPHABET.get_or_init(|| compute_alphabet(FONTS[0].bytes))
}

/// Returns whether `size` is a valid generated-case font size: within
/// `[MIN_FONT_SIZE, MAX_FONT_SIZE]`, on the [`FONT_SIZE_STEP`] grid, and not one Blink's
/// `FreeType` ascent/descent hack (see [`hack_would_fire`]) fires for on the bundled
/// font.
///
/// This is exactly the check [`sample_font_size`]'s reroll loop applies, factored out so
/// the minimiser (`src/minimise.rs`) can validate a candidate size without resampling.
#[must_use]
pub fn valid_font_size(size: f32) -> bool {
    if !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&size) {
        return false;
    }
    if !is_on_grid(size, 1.0 / FONT_SIZE_STEP) {
        return false;
    }
    let (hhea_descender, units_per_em) = roboto_metrics();
    !hack_would_fire(hhea_descender, units_per_em, size)
}

/// Unicode-hazard-class ranges excluded from the sampling alphabet: `Cc`, `Zs`, `Cf`,
/// then `Co`, in that order, matching the table in the Phase 1 doc's "Alphabet"
/// section.
const HAZARD_RANGES: &[(u32, u32)] = &[
    // Cc
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    // Zs
    (0x0020, 0x0020),
    (0x00A0, 0x00A0),
    (0x1680, 0x1680),
    (0x2000, 0x200A),
    (0x202F, 0x202F),
    (0x205F, 0x205F),
    (0x3000, 0x3000),
    // Cf
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    // Co
    (0xE000, 0xF8FF),
];

/// Well-known RTL and complex-script Unicode block ranges, used only as a coarse
/// sanity check (not authoritative Unicode bidi-class/script data) that the bundled
/// font's `cmap` has none of them — see "Alphabet" in the Phase 1 doc.
const RTL_AND_COMPLEX_SCRIPT_BLOCKS: &[(u32, u32)] = &[
    (0x0590, 0x08FF), // Hebrew .. Arabic Extended-A
    (0x0900, 0x0DFF), // Devanagari .. Sinhala
    (0x0E00, 0x0E7F), // Thai
    (0x0E80, 0x0EFF), // Lao
    (0x0F00, 0x0FFF), // Tibetan
    (0x1000, 0x109F), // Myanmar
    (0x1780, 0x17FF), // Khmer
    (0xFB50, 0xFDFF), // Arabic Presentation Forms-A
    (0xFE70, 0xFEFF), // Arabic Presentation Forms-B
];

fn in_ranges(codepoint: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .iter()
        .any(|&(lo, hi)| codepoint >= lo && codepoint <= hi)
}

/// Computes the sampling alphabet from a font's raw bytes: its `cmap` intersected with
/// the BMP, minus the hazard classes in [`HAZARD_RANGES`].
fn compute_alphabet(bytes: &[u8]) -> Vec<char> {
    let font = FontRef::new(bytes).expect("bundled font parses");
    let cmap = font.cmap().expect("bundled font has cmap");
    let (_, _, subtable) = cmap
        .best_subtable()
        .expect("bundled font has a usable cmap subtable");

    let mut alphabet: Vec<char> = subtable
        .iter()
        .filter_map(|(codepoint, _glyph_id)| char::from_u32(codepoint))
        .filter(|&ch| u32::from(ch) <= 0xFFFF && !in_ranges(u32::from(ch), HAZARD_RANGES))
        .collect();
    alphabet.sort_unstable();
    alphabet.dedup();

    for &ch in &alphabet {
        assert!(
            !in_ranges(u32::from(ch), RTL_AND_COMPLEX_SCRIPT_BLOCKS),
            "alphabet contains U+{:04X}, which falls in an RTL/complex-script block; v1 \
             assumes the bundled font has none (see \"Alphabet\" in the Phase 1 doc)",
            u32::from(ch)
        );
    }

    alphabet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        for seed in 0..1024 {
            assert_eq!(
                Case::from_seed(seed),
                Case::from_seed(seed),
                "seed {seed}: generation is not deterministic"
            );
        }
    }
}
