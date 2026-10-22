// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Frozen eligibility set for automatic CJ justification.
//!
//! Attribution: derived from Chromium's Blink (Copyright The Chromium Authors,
//! BSD-style license), revision 05991e7c3233d1e23e57f2b537396813c383f273:
//! <https://chromium.googlesource.com/chromium/src/+/05991e7c3233d1e23e57f2b537396813c383f273/third_party/blink/renderer/platform/text/character_property_data.h>
//! and `character_property_data_generator.cc` in the same directory.
//!
//! This is the evaluated union of `kIsCjkIdeographOrSymbolArray`,
//! `kIsCjkIdeographOrSymbolRanges`, `Emoji_Presentation`, and `Extended_Pictographic`
//! characters participating in RGI emoji ZWJ/modifier sequences. The emoji inputs
//! are Unicode 17.0: Chromium's vendored `icu_properties_data-v2` inversion lists
//! (`Emoji_Presentation` and `Extended_Pictographic`), and the fully-qualified ZWJ
//! and modifier sequences in `unicode-width-v0_2/tests/emoji-test.txt`.
//! The latter is Copyright 2025 Unicode, Inc. (<https://www.unicode.org/terms_of_use.html>).
//! No Unicode-property approximation or runtime emoji expansion is used here.
//! In particular U+3000 is included, while Hangul syllables are not.

pub(super) fn is_justify_expandable(ch: char) -> bool {
    let cp = u32::from(ch);
    let index = RANGES.partition_point(|&(_, end)| end < cp);
    RANGES.get(index).is_some_and(|&(start, _)| start <= cp)
}

// 105138 code points, coalesced into 133 sorted, inclusive ranges.
const RANGES: &[(u32, u32)] = &[
    (0x2C7, 0x2C7),
    (0x2CA, 0x2CB),
    (0x2D9, 0x2D9),
    (0x2020, 0x2021),
    (0x2030, 0x2030),
    (0x203B, 0x203C),
    (0x2042, 0x2042),
    (0x2047, 0x2049),
    (0x2051, 0x2051),
    (0x20DD, 0x20DE),
    (0x2100, 0x2100),
    (0x2103, 0x2103),
    (0x2105, 0x2105),
    (0x2109, 0x210A),
    (0x2113, 0x2113),
    (0x2116, 0x2116),
    (0x2121, 0x2121),
    (0x212B, 0x212B),
    (0x213B, 0x213B),
    (0x2150, 0x2152),
    (0x2156, 0x215A),
    (0x2160, 0x216B),
    (0x2170, 0x217B),
    (0x217F, 0x217F),
    (0x2189, 0x2189),
    (0x2194, 0x2195),
    (0x2307, 0x2307),
    (0x2312, 0x2312),
    (0x231A, 0x231B),
    (0x23BE, 0x23CC),
    (0x23CE, 0x23CE),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x2423, 0x2423),
    (0x2460, 0x2492),
    (0x249C, 0x24FF),
    (0x25A0, 0x25A2),
    (0x25AA, 0x25AB),
    (0x25B1, 0x25B3),
    (0x25B6, 0x25B7),
    (0x25BC, 0x25BD),
    (0x25C0, 0x25C1),
    (0x25C6, 0x25C7),
    (0x25C9, 0x25C9),
    (0x25CB, 0x25CC),
    (0x25CE, 0x25D3),
    (0x25E2, 0x25E6),
    (0x25EF, 0x25EF),
    (0x25FD, 0x25FE),
    (0x2600, 0x2603),
    (0x2605, 0x2606),
    (0x260E, 0x260E),
    (0x2614, 0x2617),
    (0x261D, 0x261D),
    (0x2620, 0x2620),
    (0x2640, 0x2640),
    (0x2642, 0x2642),
    (0x2648, 0x2653),
    (0x2660, 0x266F),
    (0x2672, 0x267D),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x2695, 0x2696),
    (0x26A0, 0x26A1),
    (0x26A7, 0x26A7),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D3, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26F9, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x2708, 0x2708),
    (0x270A, 0x270D),
    (0x2713, 0x2713),
    (0x271A, 0x271A),
    (0x2728, 0x2728),
    (0x273F, 0x2740),
    (0x2744, 0x2744),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2757),
    (0x2763, 0x2764),
    (0x2776, 0x277F),
    (0x2795, 0x2797),
    (0x27A1, 0x27A1),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1A, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x2FDF),
    (0x2FF0, 0x302D),
    (0x3031, 0x312F),
    (0x3190, 0x31EF),
    (0x3200, 0x9FFF),
    (0xF860, 0xF862),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE12),
    (0xFE19, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF0C),
    (0xFF0E, 0xFF1A),
    (0xFF1D, 0xFF1D),
    (0xFF1F, 0xFFEF),
    (0x16FE0, 0x18AFF),
    (0x1B000, 0x1B12F),
    (0x1B170, 0x1B2FF),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F100, 0x1F100),
    (0x1F110, 0x1F129),
    (0x1F130, 0x1F149),
    (0x1F150, 0x1F169),
    (0x1F170, 0x1F189),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F1E6, 0x1F6FF),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F900, 0x1F9FF),
    (0x1FA70, 0x1FA7C),
    (0x1FA80, 0x1FA8A),
    (0x1FA8E, 0x1FAC6),
    (0x1FAC8, 0x1FADC),
    (0x1FADF, 0x1FAEA),
    (0x1FAEF, 0x1FAF8),
    (0x20000, 0x2FFFF),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_chromium_set() {
        assert!(RANGES.windows(2).all(|w| w[0].1 + 1 < w[1].0));
        assert!(RANGES.iter().all(|&(start, end)| start <= end));
        assert_eq!(RANGES.iter().map(|&(a, z)| z - a + 1).sum::<u32>(), 105138);
        for ch in [
            '漢',
            '字',
            '典',
            '、',
            '。',
            'あ',
            'ア',
            '\u{3000}',
            '\u{2C7}',
            '😀',
            '♀',
            '\u{1FAE9}',
            '\u{20000}',
            '\u{2FFFF}',
        ] {
            assert!(is_justify_expandable(ch), "{ch:?}");
        }
        for ch in [
            'a',
            ' ',
            '\u{A0}',
            '.',
            '한',
            '\u{302E}',
            '\u{302F}',
            '\u{3030}',
            '\u{200D}',
            '\u{FE0F}',
            '\u{30000}',
        ] {
            assert!(!is_justify_expandable(ch), "{ch:?}");
        }
    }
}
