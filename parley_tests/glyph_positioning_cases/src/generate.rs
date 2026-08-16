// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::sync::OnceLock;

use rand::{RngExt, SeedableRng, seq::IndexedRandom};
use rand_chacha::ChaCha8Rng;
use read_fonts::{FontRef, TableProvider};

use crate::FONTS;
use crate::chromium_quantization::{SPACING_GRID_STEPS_PER_PX, floor_to_layout_unit, is_on_grid};
use crate::freetype_hack::hack_would_fire;

/// A glyph-positioning test case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// Seed from which the case was generated.
    pub seed: u64,
    /// Styled text runs in source order.
    pub runs: Vec<Run>,
    /// Container width in CSS pixels.
    pub width: f32,
}

/// A styled text run in a [`Case`].
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Text content.
    pub text: String,
    /// Font size in CSS pixels.
    pub font_size: f32,
    /// Letter spacing in CSS pixels.
    pub letter_spacing: f32,
    /// Word spacing in CSS pixels.
    pub word_spacing: f32,
    /// Absolute line height in CSS pixels.
    pub line_height: f32,
}

const MIN_RUNS: usize = 1;
const MAX_RUNS: usize = 4;

const MIN_TOTAL_LEN: usize = 30;
const MAX_TOTAL_LEN: usize = 120;

const SPACE_PROBABILITY: f64 = 0.18;

pub(crate) const MIN_FONT_SIZE: f32 = 10.0;
pub(crate) const MAX_FONT_SIZE: f32 = 30.0;

pub(crate) const FONT_SIZE_STEP: f32 = 0.25;
#[expect(
    clippy::cast_possible_truncation,
    reason = "the font-size range has only 80 steps"
)]
const FONT_SIZE_STEPS: u32 = ((MAX_FONT_SIZE - MIN_FONT_SIZE) / FONT_SIZE_STEP) as u32;

const MIN_EM_FACTOR: f32 = 7.0;
const MAX_EM_FACTOR: f32 = 32.0;

/// Maximum generated container width in CSS pixels.
pub const MAX_CASE_WIDTH_PX: f32 = 1000.0;

const LETTER_SPACING_RANGE_PX: (f32, f32) = (0.0, 0.0);
const WORD_SPACING_RANGE_PX: (f32, f32) = (0.0, 0.0);

const LINE_HEIGHT_FACTOR_RANGE: (f32, f32) = (0.8, 2.0);

impl Case {
    /// Generates the case for `seed`.
    #[must_use]
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let alphabet = alphabet();

        let num_runs = rng.random_range(MIN_RUNS..=MAX_RUNS);
        let total_len = rng.random_range(MIN_TOTAL_LEN..=MAX_TOTAL_LEN);
        let lengths = partition_length(&mut rng, total_len, num_runs);

        let mut prev_was_space = true;
        let mut previous_font_size = None;
        let runs: Vec<Run> = lengths
            .into_iter()
            .map(|len| {
                let text = generate_run_text(&mut rng, alphabet, len, &mut prev_was_space);
                let font_size = sample_font_size(&mut rng, previous_font_size);
                previous_font_size = Some(font_size);
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

fn partition_length(rng: &mut ChaCha8Rng, total_len: usize, num_runs: usize) -> Vec<usize> {
    let mut lengths = vec![1_usize; num_runs];
    for _ in 0..(total_len - num_runs) {
        let index = rng.random_range(0..num_runs);
        lengths[index] += 1;
    }
    lengths
}

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

fn sample_font_size(rng: &mut ChaCha8Rng, previous: Option<f32>) -> f32 {
    let (hhea_descender, units_per_em) = roboto_metrics();
    loop {
        let size = MIN_FONT_SIZE + rng.random_range(0..=FONT_SIZE_STEPS) as f32 * FONT_SIZE_STEP;
        if hack_would_fire(hhea_descender, units_per_em, size) {
            continue;
        }
        if previous == Some(size) {
            continue;
        }
        return size;
    }
}

fn sample_line_height(rng: &mut ChaCha8Rng, font_size: f32) -> f32 {
    let (min_factor, max_factor) = LINE_HEIGHT_FACTOR_RANGE;
    floor_to_layout_unit(font_size * rng.random_range(min_factor..=max_factor))
}

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

fn roboto_metrics() -> (i16, u16) {
    static METRICS: OnceLock<(i16, u16)> = OnceLock::new();
    *METRICS.get_or_init(|| {
        let font = FontRef::new(FONTS[0].bytes).expect("bundled font parses");
        let hhea = font.hhea().expect("bundled font has hhea");
        let head = font.head().expect("bundled font has head");
        (hhea.descender().to_i16(), head.units_per_em())
    })
}

#[must_use]
/// Returns the characters used by generated cases.
pub fn alphabet() -> &'static [char] {
    static ALPHABET: OnceLock<Vec<char>> = OnceLock::new();
    ALPHABET.get_or_init(|| compute_alphabet(FONTS[0].bytes))
}

#[must_use]
/// Reports whether a font size satisfies the generator's constraints.
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

// Controls, unusual spaces, and formatting characters make the recorded text ambiguous.
const HAZARD_RANGES: &[(u32, u32)] = &[
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    (0x0020, 0x0020),
    (0x00A0, 0x00A0),
    (0x1680, 0x1680),
    (0x2000, 0x200A),
    (0x202F, 0x202F),
    (0x205F, 0x205F),
    (0x3000, 0x3000),
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
    (0xE000, 0xF8FF),
];

// Glyphs are compared in emission order, so exclude scripts requiring bidi or reordering.
const RTL_AND_COMPLEX_SCRIPT_BLOCKS: &[(u32, u32)] = &[
    (0x0590, 0x08FF),
    (0x0900, 0x0DFF),
    (0x0E00, 0x0E7F),
    (0x0E80, 0x0EFF),
    (0x0F00, 0x0FFF),
    (0x1000, 0x109F),
    (0x1780, 0x17FF),
    (0xFB50, 0xFDFF),
    (0xFE70, 0xFEFF),
];

fn in_ranges(codepoint: u32, ranges: &[(u32, u32)]) -> bool {
    ranges
        .iter()
        .any(|&(lo, hi)| codepoint >= lo && codepoint <= hi)
}

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
            "alphabet contains unsupported RTL or complex-script character U+{:04X}",
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
