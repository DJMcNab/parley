// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! See `./main.rs`.

use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{
        BidiClass, DefaultIgnorableCodePoint, EastAsianWidth, Emoji, EmojiModifier,
        EmojiModifierBase, EmojiPresentation, ExtendedPictographic, GeneralCategory,
        GraphemeClusterBreak, LineBreak, Script, VariationSelector,
    },
};
use parley_data::{Properties, TextSpacingTrimClass};
use std::io::{BufWriter, Write};

const COPYRIGHT_HEADER: &str =
    "// Copyright 2025 the Parley Authors\n// SPDX-License-Identifier: Apache-2.0 OR MIT\n";

/// Generation configuration.
#[derive(Debug)]
pub struct Config {
    /// Compression level (1.0 = balanced, 5.0 = smaller, 9.0 = even smaller, 10.0 = smallest).
    pub compression: f64,
    /// Whether to use unsafe array access in generated code.
    pub unsafe_access: bool,
}

/// Exports ICU data as `PackTab` lookup tables + generated Rust code into the `out` directory.
pub fn generate(out: std::path::PathBuf, config: &Config) {
    let emoji_data = CodePointSetData::new::<Emoji>();
    let extended_pictographic_data = CodePointSetData::new::<ExtendedPictographic>();

    let script_data = CodePointMapData::<Script>::new();
    let general_category_data = CodePointMapData::<GeneralCategory>::new();
    let grapheme_cluster_break_data = CodePointMapData::<GraphemeClusterBreak>::new();
    let bidi_class_data = CodePointMapData::<BidiClass>::new();

    let variation_selector_data = CodePointSetData::new::<VariationSelector>();
    let default_ignorable_data = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let line_break_data = CodePointMapData::<LineBreak>::new();

    let emoji_presentation_data = CodePointSetData::new::<EmojiPresentation>();
    let emoji_modifier_data = CodePointSetData::new::<EmojiModifier>();
    let emoji_modifier_base_data = CodePointSetData::new::<EmojiModifierBase>();

    let east_asian_width_data = CodePointMapData::<EastAsianWidth>::new();

    // Generate the data required for `CompositeProps`.
    // Dense characters table for 0..=0x10FFFF
    let mut characters = Vec::with_capacity(0x110000);

    for cp in 0_u32..=0x10FFFF {
        let is_emoji = emoji_data.contains32(cp);
        let is_extended_pictographic = extended_pictographic_data.contains32(cp);

        // This encodes in which instances ICU4X uses a dictionary to determine a word break instead
        // of applying default UAX #29 rules; if ICU4X lacks a dictionary, it emits a sequence as a
        // single word. Unicode recommends tailoring for those code points, but which exact set this
        // tracks is an ICU4X implementation detail that we try to match.
        let needs_dictionary_word_break = line_break_data.get32(cp) == LineBreak::ComplexContext
            || matches!(script_data.get32(cp), Script::Han | Script::Hiragana);

        // See https://drafts.csswg.org/css-text-4/#text-spacing-classes
        let text_spacing_trim_class = {
            // The characters that opening and closing punctuation must be among to be fullwidth:
            // those in the CJK Symbols and Punctuation block, or with an East Asian Width of
            // Fullwidth.
            let is_cjk_symbol_or_fullwidth = (0x3000..=0x303F).contains(&cp)
                || east_asian_width_data.get32(cp) == EastAsianWidth::Fullwidth;
            match cp {
                // Fullwidth dot punctuation.
                0x3001 | 0x3002 | 0xFF0C | 0xFF0E => TextSpacingTrimClass::Dot,
                // Fullwidth colon punctuation.
                0xFF1A | 0xFF1B => TextSpacingTrimClass::Colon,
                // Fullwidth middle dot punctuation.
                0x00B7 | 0x2027 | 0x30FB => TextSpacingTrimClass::Middle,
                // The ideographic space isn't punctuation, but opening and closing punctuation
                // next to it is trimmed just as it is next to a middle dot.
                0x3000 => TextSpacingTrimClass::Middle,
                // The curly quotes are shared between Latin, where they're narrow, and CJK, where
                // they're often fullwidth. They are initial and final punctuation (`Pi` and
                // `Pf`), not opening and closing (`Ps` and `Pe`), so they're listed explicitly.
                0x2018 | 0x201C => TextSpacingTrimClass::Open,
                0x2019 | 0x201D => TextSpacingTrimClass::Close,
                _ => match (general_category_data.get32(cp), is_cjk_symbol_or_fullwidth) {
                    (GeneralCategory::OpenPunctuation, true) => TextSpacingTrimClass::Open,
                    (GeneralCategory::OpenPunctuation, false) => TextSpacingTrimClass::NarrowOpen,
                    (GeneralCategory::ClosePunctuation, true) => TextSpacingTrimClass::Close,
                    (GeneralCategory::ClosePunctuation, false) => TextSpacingTrimClass::NarrowClose,
                    _ => TextSpacingTrimClass::Other,
                },
            }
        };

        let v = Properties::new(
            script_data.get32(cp),
            general_category_data.get32(cp),
            grapheme_cluster_break_data.get32(cp),
            bidi_class_data.get32(cp),
            is_emoji || is_extended_pictographic,
            variation_selector_data.contains32(cp),
            default_ignorable_data.contains32(cp),
            // See: https://github.com/unicode-org/icu4x/blob/ee5399a77a6b94efb5d4b60678bb458c5eedb25d/components/segmenter/src/line.rs#L338-L351
            matches!(
                line_break_data.get32(cp),
                LineBreak::MandatoryBreak
                    | LineBreak::CarriageReturn
                    | LineBreak::LineFeed
                    | LineBreak::NextLine
            ),
            needs_dictionary_word_break,
            is_emoji,
            emoji_presentation_data.contains32(cp),
            emoji_modifier_data.contains32(cp),
            emoji_modifier_base_data.contains32(cp),
            text_spacing_trim_class,
        );
        characters.push(u64::from(v).cast_signed());
    }

    let (info, best) = packtab::pack_table(&characters, Some(0), config.compression);

    let namespace = "composite_packtab";
    let mut code = packtab::generate(
        &info,
        best,
        namespace,
        packtab::codegen::Language::Rust {
            unsafe_access: config.unsafe_access,
        },
    );

    if !code.ends_with('\n') {
        code.push('\n');
    }

    let mut file = BufWriter::new(std::fs::File::create(out.join("mod.rs")).unwrap());
    writeln!(&mut file, "{COPYRIGHT_HEADER}").unwrap();
    writeln!(
        &mut file,
        "//! Backing data for composite properties (PackTab, compression={}, unsafe={})",
        config.compression, config.unsafe_access
    )
    .unwrap();
    writeln!(&mut file).unwrap();
    write!(&mut file, "{code}").unwrap();
}
