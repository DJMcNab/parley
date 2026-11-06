// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Diagnostic probes for Unicode-range shaping callbacks.

use super::*;
use alloc::{sync::Arc, vec, vec::Vec};
use fontique::UnicodeRangeSet;
use harfrust::{Advances, Buffer, Direction, FontFuncs, ShaperFont};

struct Passthrough;
impl FontFuncs for Passthrough {}

struct BatchedPassthrough;
impl FontFuncs for BatchedPassthrough {
    fn glyph_h_advances(&self, font: &ShaperFont<'_, '_>, advances: Advances<'_>) {
        font.default_glyph_h_advances(advances);
    }
}

struct DefaultRangeMetrics<'a>(RangeFontFuncs<'a>);
impl FontFuncs for DefaultRangeMetrics<'_> {
    fn nominal_glyph(&self, font: &ShaperFont<'_, '_>, c: u32) -> Option<harfrust::GlyphId> {
        self.0.nominal_glyph(font, c)
    }
    fn variation_glyph(
        &self,
        font: &ShaperFont<'_, '_>,
        c: u32,
        vs: u32,
    ) -> Option<harfrust::GlyphId> {
        self.0.variation_glyph(font, c, vs)
    }
}

/// `cargo test -p parley_engine --release benchmark_unicode_range_callbacks -- --ignored --nocapture`
#[test]
#[ignore = "manual performance measurement"]
fn benchmark_unicode_range_callbacks() {
    const ROBOTO: &[u8] =
        include_bytes!("../../../../parley_dev/assets/fonts/roboto_fonts/Roboto-Regular.ttf");
    measure_callbacks(
        ROBOTO,
        "The quick brown fox jumps over the lazy dog. ",
        Direction::LeftToRight,
        harfrust::Script::LATIN,
        0x7E,
    );
}

#[test]
#[ignore = "manual performance measurement"]
fn benchmark_unicode_range_callbacks_arabic() {
    const ARABIC: &[u8] =
        include_bytes!("../../../../parley_dev/assets/fonts/noto_fonts/NotoKufiArabic-Regular.otf");
    measure_callbacks(
        ARABIC,
        "اللغة العربية جميلة وتكتب من اليمين إلى اليسار ",
        Direction::RightToLeft,
        harfrust::Script::ARABIC,
        0x6FF,
    );
}

#[expect(clippy::print_stdout, reason = "Reports manual benchmark measurements")]
fn measure_callbacks(
    bytes: &'static [u8],
    sample: &str,
    direction: Direction,
    script: harfrust::Script,
    end: u32,
) {
    use std::{hint::black_box, println, time::Instant};

    let text = sample.repeat(20);
    let (data, _) = Blob::new(Arc::new(bytes)).into_raw_parts();
    let font = harfrust::Font::new(data, 0).unwrap();
    let one = UnicodeRangeSet::new(&mut vec![(0x20, end)]);
    let many = UnicodeRangeSet::new(
        &mut core::iter::once((0x20, end))
            .chain((0..128).map(|i| (0x1000 + i * 2, 0x1000 + i * 2)))
            .collect(),
    );
    let passthrough = Passthrough;
    let batched_passthrough = BatchedPassthrough;
    let default_range = DefaultRangeMetrics(RangeFontFuncs(&one));
    let one_funcs = RangeFontFuncs(&one);
    let many_funcs = RangeFontFuncs(&many);
    let mut buffer = Buffer::new();
    const N: usize = 6;
    let funcs: [Option<&dyn FontFuncs>; N] = [
        None,
        Some(&passthrough),
        Some(&batched_passthrough),
        Some(&default_range),
        Some(&one_funcs),
        Some(&many_funcs),
    ];
    let names = [
        "unrestricted",
        "passthrough / default metrics",
        "passthrough / batched metrics",
        "one range / default metrics",
        "one range / batched metrics",
        "129 ranges / batched metrics",
    ];
    let mut samples: [Vec<f64>; N] = core::array::from_fn(|_| Vec::new());
    let mut expected = None;
    const ITERATIONS: u32 = 1500;

    for round in 0..9 {
        for offset in 0..N {
            let case = (offset + round) % N;
            let start = Instant::now();
            for _ in 0..ITERATIONS {
                buffer.clear();
                buffer.push_str(black_box(&text));
                buffer.set_direction(direction);
                buffer.set_script(Some(script));
                let shaper_font = ShaperFont::new(&font).with_font_funcs(funcs[case]);
                harfrust::shape(&shaper_font, &mut buffer, HarfShapeOptions::new()).unwrap();
                black_box(buffer.glyph_infos());
                black_box(buffer.glyph_positions());
            }
            samples[case].push(start.elapsed().as_secs_f64() * 1e9 / f64::from(ITERATIONS));

            let output: Vec<_> = buffer
                .glyph_infos()
                .iter()
                .zip(buffer.glyph_positions())
                .map(|(g, p)| (g.glyph_id, p.x_advance, p.y_advance, p.x_offset, p.y_offset))
                .collect();
            if let Some(expected) = &expected {
                assert_eq!(
                    &output, expected,
                    "callback variants must preserve glyphs and positions"
                );
            } else {
                expected = Some(output);
            }
        }
    }
    let medians = samples.map(|mut values| {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    });
    for (name, ns) in names.into_iter().zip(medians) {
        println!(
            "{name}: {ns:.0} ns/shape ({:+.1}% vs unrestricted)",
            (ns / medians[0] - 1.0) * 100.0
        );
    }
}
