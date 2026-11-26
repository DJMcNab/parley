// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! CSS `text-spacing-trim`: setting fullwidth punctuation half-width.
//!
//! Many fullwidth CJK punctuation glyphs only draw in half of their box; the other half is blank.
//! [CSS Text 4][spec] defines when such a glyph is set *spaced* (full-width) or *trimmed*
//! (half-width, with the blank half removed). Parley trims a character by requesting the OpenType
//! `halt` feature for it: the font decides whether and how its glyph changes.
//!
//! Which characters to trim depends on where lines break, so this works in two steps:
//!
//! 1. Before shaping, [`TrimSites::collect`] finds the characters that are trimmed because of the
//!    character next to them, and the paragraph is shaped with [those](TrimSites::trims) trimmed.
//! 2. The line breaker reshapes the edges of a line where that no longer holds, using
//!    [`TrimSites::line_overrides`].
//!
//! [spec]: https://drafts.csswg.org/css-text-4/#text-spacing-trim-property

use alloc::vec::Vec;
use core::range::Range;

use parlance::{BidiLevel, FontFeature, Language, Tag};
use parley_data::{Properties, TextSpacingTrimClass};
use parley_engine::RangedFeature;

use crate::resolve::{ResolveContext, ResolvedStyle};
use crate::style::Brush;

/// Where fullwidth punctuation is set half-width, with the blank half of its box trimmed.
///
/// This is the CSS [`text-spacing-trim`] property, which has one setting for each of the positions
/// punctuation can be trimmed at. Only some of the CSS values are supported so far.
///
/// Trimming is done by the font's `halt` feature, so it has no effect in fonts without it.
/// Parley's request to trim a character takes precedence over that character's `halt` setting in
/// [`FontFeatures`](crate::FontFeatures). Note that a character Parley does *not* trim keeps that
/// setting: [`Self::SPACE_ALL`] doesn't turn `halt` off.
///
/// [`text-spacing-trim`]: https://drafts.csswg.org/css-text-4/#text-spacing-trim-property
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextSpacingTrim {
    /// Opening punctuation at the start of a line.
    pub start: TextSpacingTrimStart,
    /// Punctuation next to other punctuation.
    pub adjacent: TextSpacingTrimAdjacent,
    /// Closing punctuation at the end of a line.
    pub end: TextSpacingTrimEnd,
}

impl TextSpacingTrim {
    /// Don't trim anything (CSS `space-all`). This is the default.
    pub const SPACE_ALL: Self = Self {
        start: TextSpacingTrimStart::Space,
        adjacent: TextSpacingTrimAdjacent::Space,
        end: TextSpacingTrimEnd::Space,
    };
    /// Trim adjacent punctuation, and closing punctuation that otherwise doesn't fit at the end
    /// of its line (CSS `normal`).
    pub const NORMAL: Self = Self {
        start: TextSpacingTrimStart::Space,
        adjacent: TextSpacingTrimAdjacent::Trim,
        end: TextSpacingTrimEnd::Allow,
    };
}

/// Whether opening punctuation at the start of a line is trimmed. See [`TextSpacingTrim`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextSpacingTrimStart {
    /// Set it full-width (CSS `space-start`).
    #[default]
    Space,
}

/// Whether punctuation next to other punctuation is trimmed. See [`TextSpacingTrim`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextSpacingTrimAdjacent {
    /// Set it full-width (CSS `space-adjacent`).
    #[default]
    Space,
    /// Collapse the blank space between the two (CSS `trim-adjacent`).
    Trim,
}

/// Whether closing punctuation at the end of a line is trimmed. See [`TextSpacingTrim`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextSpacingTrimEnd {
    /// Set it full-width (CSS `space-end`).
    #[default]
    Space,
    /// Trim it if the line doesn't fit otherwise (CSS `allow-end`).
    Allow,
}

/// The feature that sets `char` half-width (if `value` is 1), or leaves it full-width (0).
fn half_width(char: u32, value: u16) -> RangedFeature {
    RangedFeature {
        feature: FontFeature::new(HALF_WIDTH_ALTERNATES, value),
        char_range: (char..char + 1).into(),
    }
}

/// The OpenType feature for the half-width forms of fullwidth glyphs.
///
/// Vertical text would use `vhal` instead.
const HALF_WIDTH_ALTERNATES: Tag = Tag::new(b"halt");

/// The neighbor of a character that causes it to be trimmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Neighbor {
    Previous,
    Next,
}

/// A character that `text-spacing-trim` trims, or may trim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrimSite {
    /// The index of the character in the source text.
    char: u32,
    /// The neighbor because of which the paragraph was shaped with this character trimmed, if it
    /// was. The trim only holds on lines that have both characters.
    trimmed_by: Option<Neighbor>,
    /// The value of `halt` in the character's style, which is what undoing its trim restores.
    style_half_width: u16,
    /// Whether this is closing punctuation that may be trimmed if its line ends with it and
    /// doesn't fit otherwise (`allow-end`).
    allow_end: bool,
}

/// The characters of a paragraph that `text-spacing-trim` trims or may trim, in source order.
///
/// These are the characters Parley *asks* the font to trim; the font may not support that.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TrimSites(Vec<TrimSite>);

impl TrimSites {
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    /// Find the trim sites of `text`, replacing the current ones.
    ///
    /// `bidi_levels` is as returned by [`parley_engine::Analysis::bidi_levels`].
    pub(crate) fn collect<B: Brush>(
        &mut self,
        text: &str,
        rcx: &ResolveContext,
        styles: &[ResolvedStyle<B>],
        char_style_indices: &[u16],
        bidi_levels: &[BidiLevel],
    ) {
        self.clear();
        // `space-all` leaves everything as shaping does it anyway.
        if (styles.iter()).all(|style| style.text_spacing_trim == TextSpacingTrim::SPACE_ALL) {
            return;
        }

        let mut classes = text
            .chars()
            .map(|ch| Properties::get(ch).text_spacing_trim_class())
            .enumerate()
            .peekable();
        let mut previous = TextSpacingTrimClass::Other;
        while let Some((char, raw_class)) = classes.next() {
            let next = classes.peek().map_or(TextSpacingTrimClass::Other, |c| c.1);
            let style = &styles[usize::from(char_style_indices[char])];
            let conventions = PunctuationConventions::for_language(style.locale.as_ref());
            // A neighbor's class is resolved with the conventions of the character it may cause
            // to be trimmed. Whether the two differ in style, or end up in different shaped runs,
            // doesn't matter.
            let neighbor = |raw_class, index: usize| {
                // TODO: A character in the other direction is not treated as a neighbor at all:
                // across a direction change, the characters that are next to each other visually
                // are not the ones that are in the text.
                if bidi_levels.get(index) == bidi_levels.get(char) {
                    TrimClass::resolve(raw_class, conventions)
                } else {
                    TrimClass::Other
                }
            };
            let class = TrimClass::resolve(raw_class, conventions);
            let trim = style.text_spacing_trim;
            let trimmed_by = match trim.adjacent {
                TextSpacingTrimAdjacent::Space => None,
                TextSpacingTrimAdjacent::Trim => {
                    if class.is_trimmed_after(neighbor(previous, char.saturating_sub(1))) {
                        Some(Neighbor::Previous)
                    } else if class.is_trimmed_before(neighbor(next, char + 1)) {
                        Some(Neighbor::Next)
                    } else {
                        None
                    }
                }
            };
            let allow_end = class == TrimClass::Close && trim.end == TextSpacingTrimEnd::Allow;
            if trimmed_by.is_some() || allow_end {
                let style_features = rcx.features(style.font_features).unwrap_or(&[]);
                // Later features override earlier features.
                let style_half_width = (style_features.iter().rev())
                    .find(|feature| feature.tag == HALF_WIDTH_ALTERNATES)
                    .map_or(0, |feature| feature.value);
                self.0.push(TrimSite {
                    char: char as u32,
                    trimmed_by,
                    style_half_width,
                    allow_end,
                });
            }
            previous = raw_class;
        }
    }

    /// The features that trim the characters that are trimmed because of a neighbor, sorted by
    /// character.
    pub(crate) fn trims(&self) -> impl Iterator<Item = RangedFeature> {
        (self.0.iter())
            .filter(|site| site.trimmed_by.is_some())
            .map(|site| half_width(site.char, 1))
    }

    /// The feature to reshape the start of a line with, if it starts at `line_start_char`.
    ///
    /// The line's first character is no longer trimmed if that was because of the character
    /// before it, which is on another line.
    pub(crate) fn line_start_override(&self, line_start_char: u32) -> Option<RangedFeature> {
        self.untrim(line_start_char, Neighbor::Previous)
    }

    /// The feature to reshape the end of a line with, if it ends before `line_end_char`.
    ///
    /// The line's end trims the character `trimmed_end`, if any, which must
    /// [allow](Self::allows_end_trim) that. Otherwise, the line's last character is no longer
    /// trimmed if that was because of the character after it, which is on another line.
    pub(crate) fn line_end_override(
        &self,
        line_end_char: u32,
        trimmed_end: Option<u32>,
    ) -> Option<RangedFeature> {
        match trimmed_end {
            Some(char) => Some(half_width(char, 1)),
            None => self.untrim(line_end_char.checked_sub(1)?, Neighbor::Next),
        }
    }

    /// The features to reshape the start and end of the line of characters `chars` with; see
    /// [`Self::line_start_override`] and [`Self::line_end_override`].
    pub(crate) fn line_overrides(
        &self,
        chars: Range<u32>,
        trimmed_end: Option<u32>,
    ) -> [Option<RangedFeature>; 2] {
        [
            self.line_start_override(chars.start),
            self.line_end_override(chars.end, trimmed_end),
        ]
    }

    /// The feature that undoes the trim of `char`, if it was trimmed because of `neighbor`.
    fn untrim(&self, char: u32, neighbor: Neighbor) -> Option<RangedFeature> {
        let site = self.get(char)?;
        (site.trimmed_by == Some(neighbor)).then(|| half_width(char, site.style_half_width))
    }

    /// Whether `char` is closing punctuation that may be trimmed if its line ends with it and
    /// doesn't fit otherwise.
    pub(crate) fn allows_end_trim(&self, char: u32) -> bool {
        self.get(char).is_some_and(|site| site.allow_end)
    }

    fn get(&self, char: u32) -> Option<&TrimSite> {
        let index = self.0.binary_search_by_key(&char, |site| site.char).ok()?;
        Some(&self.0[index])
    }
}

/// The class of a character in a particular language, for the purposes of `text-spacing-trim`.
///
/// This is a [`TextSpacingTrimClass`] with its language-dependent classes resolved using
/// [`PunctuationConventions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrimClass {
    /// Not relevant to trimming.
    Other,
    /// Fullwidth opening punctuation, e.g. `「` or `（`.
    ///
    /// The half of the glyph's box before the ink is blank, and can be trimmed.
    Open,
    /// Fullwidth closing punctuation, e.g. `」` or `）`.
    ///
    /// The half of the glyph's box after the ink is blank, and can be trimmed.
    Close,
    /// Fullwidth punctuation with blank space on both sides, e.g. `・`, and the ideographic space.
    ///
    /// These are never trimmed, but fullwidth opening or closing punctuation facing one is.
    Middle,
    /// Opening punctuation that isn't fullwidth, e.g. `(`.
    ///
    /// These are never trimmed, but fullwidth opening punctuation following one is.
    NarrowOpen,
    /// Closing punctuation that isn't fullwidth, e.g. `)`.
    ///
    /// These are never trimmed, but fullwidth closing punctuation preceding one is.
    NarrowClose,
}

/// Where fonts for a writing system draw fullwidth dots and colons within the glyph's box.
///
/// Whether these characters have a blank half that can be trimmed depends on the glyph. CSS Text 4
/// allows deciding that by language convention, which is what this does.
///
/// Note that the specification warns that fonts, Chinese ones in particular, don't always follow
/// their language's convention. When they don't, a dot or colon is trimmed when it shouldn't be,
/// or the other way around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PunctuationConventions {
    /// Whether `、`, `。`, `，` and `．` are drawn to the starting side of their box.
    ///
    /// If so, they are [`TrimClass::Close`]; otherwise they are centered and [`TrimClass::Middle`].
    dots_are_closing: bool,
    /// Whether `：` and `；` are drawn to the starting side of their box.
    ///
    /// If so, they are [`TrimClass::Close`]; otherwise they are centered and [`TrimClass::Middle`].
    colons_are_closing: bool,
}

impl PunctuationConventions {
    /// Simplified Chinese: both dots and colons are drawn to the side.
    const SIMPLIFIED_CHINESE: Self = Self {
        dots_are_closing: true,
        colons_are_closing: true,
    };
    /// Traditional Chinese: both dots and colons are centered.
    const TRADITIONAL_CHINESE: Self = Self {
        dots_are_closing: false,
        colons_are_closing: false,
    };
    /// Japanese and Korean: dots are drawn to the side, colons are centered.
    const JAPANESE_OR_KOREAN: Self = Self {
        dots_are_closing: true,
        colons_are_closing: false,
    };

    /// The conventions for text in `language`.
    ///
    /// This follows the informative table of language conventions in
    /// [Text Spacing Trim Character Classes][classes].
    ///
    /// Chinese without a script subtag is Traditional in Taiwan, Hong Kong and Macao, and
    /// Simplified elsewhere. Text that isn't known to be Chinese gets the conventions of Japanese
    /// and Korean, which for both dots and colons is what most of the listed languages do.
    ///
    /// [classes]: https://drafts.csswg.org/css-text-4/#text-spacing-classes
    fn for_language(language: Option<&Language>) -> Self {
        let Some(language) = language.filter(|language| language.language() == "zh") else {
            return Self::JAPANESE_OR_KOREAN;
        };
        match (language.script(), language.region()) {
            (Some("Hant"), _) => Self::TRADITIONAL_CHINESE,
            (Some(_), _) => Self::SIMPLIFIED_CHINESE,
            (None, Some("TW" | "HK" | "MO")) => Self::TRADITIONAL_CHINESE,
            (None, _) => Self::SIMPLIFIED_CHINESE,
        }
    }
}

impl TrimClass {
    /// The class of a character with the given language-independent class, in text following the
    /// given conventions.
    const fn resolve(class: TextSpacingTrimClass, conventions: PunctuationConventions) -> Self {
        const fn side_or_centered(is_closing: bool) -> TrimClass {
            if is_closing {
                TrimClass::Close
            } else {
                TrimClass::Middle
            }
        }
        match class {
            TextSpacingTrimClass::Other => Self::Other,
            TextSpacingTrimClass::Open => Self::Open,
            TextSpacingTrimClass::Close => Self::Close,
            TextSpacingTrimClass::Middle => Self::Middle,
            TextSpacingTrimClass::NarrowOpen => Self::NarrowOpen,
            TextSpacingTrimClass::NarrowClose => Self::NarrowClose,
            TextSpacingTrimClass::Dot => side_or_centered(conventions.dots_are_closing),
            TextSpacingTrimClass::Colon => side_or_centered(conventions.colons_are_closing),
        }
    }

    /// Whether a character of this class is trimmed because of the character before it.
    ///
    /// Only [`Self::Open`] characters can be: they are set half-width when following another
    /// character that leaves blank space before them.
    ///
    /// This implements the first half of [Fullwidth Punctuation Collapsing][collapsing].
    ///
    /// [collapsing]: https://drafts.csswg.org/css-text-4/#fullwidth-collapsing
    const fn is_trimmed_after(self, previous: Self) -> bool {
        // TODO: The specification only trims an opening character following a closing character
        // if the closing character's font size is the same or larger. If it is smaller, the
        // closing character is trimmed instead (see `is_trimmed_before`). We don't look at font
        // sizes, so we always trim the opening character.
        matches!(self, Self::Open)
            && matches!(
                previous,
                Self::Open | Self::Middle | Self::Close | Self::NarrowOpen
            )
    }

    /// Whether a character of this class is trimmed because of the character after it.
    ///
    /// Only [`Self::Close`] characters can be: they are set half-width when followed by another
    /// character that leaves blank space after them.
    ///
    /// This implements the second half of [Fullwidth Punctuation Collapsing][collapsing].
    ///
    /// [collapsing]: https://drafts.csswg.org/css-text-4/#fullwidth-collapsing
    const fn is_trimmed_before(self, next: Self) -> bool {
        // TODO: The specification also trims a closing character followed by an opening character
        // of a larger font size (see `is_trimmed_after`).
        matches!(self, Self::Close)
            && matches!(next, Self::Close | Self::Middle | Self::NarrowClose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZH_HANS: PunctuationConventions = PunctuationConventions::SIMPLIFIED_CHINESE;
    const ZH_HANT: PunctuationConventions = PunctuationConventions::TRADITIONAL_CHINESE;
    const JA: PunctuationConventions = PunctuationConventions::JAPANESE_OR_KOREAN;

    fn class(ch: char, conventions: PunctuationConventions) -> TrimClass {
        TrimClass::resolve(Properties::get(ch).text_spacing_trim_class(), conventions)
    }

    fn conventions(language: &str) -> PunctuationConventions {
        PunctuationConventions::for_language(Some(&Language::parse(language).unwrap()))
    }

    #[test]
    fn language_independent_classes() {
        for conventions in [ZH_HANS, ZH_HANT, JA] {
            assert_eq!(class('水', conventions), TrimClass::Other);
            assert_eq!(class('「', conventions), TrimClass::Open);
            assert_eq!(class('」', conventions), TrimClass::Close);
            assert_eq!(class('・', conventions), TrimClass::Middle);
            assert_eq!(class('(', conventions), TrimClass::NarrowOpen);
            assert_eq!(class(')', conventions), TrimClass::NarrowClose);
        }
    }

    #[test]
    fn dots_and_colons_follow_conventions() {
        for ch in ['、', '。', '，', '．'] {
            assert_eq!(class(ch, ZH_HANS), TrimClass::Close, "{ch}");
            assert_eq!(class(ch, ZH_HANT), TrimClass::Middle, "{ch}");
            assert_eq!(class(ch, JA), TrimClass::Close, "{ch}");
        }
        for ch in ['：', '；'] {
            assert_eq!(class(ch, ZH_HANS), TrimClass::Close, "{ch}");
            assert_eq!(class(ch, ZH_HANT), TrimClass::Middle, "{ch}");
            assert_eq!(class(ch, JA), TrimClass::Middle, "{ch}");
        }
    }

    #[test]
    fn conventions_for_language() {
        assert_eq!(PunctuationConventions::for_language(None), JA);
        assert_eq!(conventions("ja"), JA);
        assert_eq!(conventions("ko"), JA);
        assert_eq!(conventions("en"), JA);

        assert_eq!(conventions("zh"), ZH_HANS);
        assert_eq!(conventions("zh-CN"), ZH_HANS);
        assert_eq!(conventions("zh-SG"), ZH_HANS);
        assert_eq!(conventions("zh-Hans"), ZH_HANS);
        // The script subtag wins over the region.
        assert_eq!(conventions("zh-Hans-TW"), ZH_HANS);

        assert_eq!(conventions("zh-TW"), ZH_HANT);
        assert_eq!(conventions("zh-HK"), ZH_HANT);
        assert_eq!(conventions("zh-MO"), ZH_HANT);
        assert_eq!(conventions("zh-Hant"), ZH_HANT);
        assert_eq!(conventions("zh-Hant-CN"), ZH_HANT);
    }

    /// The pairs listed in the specification's "Demonstration of adjacent-pairs punctuation
    /// trimming" table.
    #[test]
    fn specification_pairs() {
        // In each of these, the second character is trimmed and the first is not.
        for (first, second) in [('〔', '（'), ('・', '（'), ('〕', '（'), ('\u{3000}', '（')]
        {
            let (first, second) = (class(first, JA), class(second, JA));
            assert!(second.is_trimmed_after(first), "{first:?} {second:?}");
            assert!(!first.is_trimmed_before(second), "{first:?} {second:?}");
        }
        // In each of these, the first character is trimmed and the second is not.
        for (first, second) in [('）', '〕'), ('）', '・'), ('）', '\u{3000}')] {
            let (first, second) = (class(first, JA), class(second, JA));
            assert!(first.is_trimmed_before(second), "{first:?} {second:?}");
            assert!(!second.is_trimmed_after(first), "{first:?} {second:?}");
        }
    }

    #[test]
    fn narrow_neighbors() {
        let (open, close) = (TrimClass::Open, TrimClass::Close);
        // Fullwidth punctuation is trimmed when nested inside narrow punctuation...
        assert!(open.is_trimmed_after(TrimClass::NarrowOpen));
        assert!(close.is_trimmed_before(TrimClass::NarrowClose));
        // ... but not when the narrow punctuation faces the other way...
        assert!(!open.is_trimmed_after(TrimClass::NarrowClose));
        assert!(!close.is_trimmed_before(TrimClass::NarrowOpen));
        // ... and narrow punctuation is itself never trimmed.
        for neighbor in [open, close, TrimClass::Middle] {
            assert!(!TrimClass::NarrowOpen.is_trimmed_after(neighbor));
            assert!(!TrimClass::NarrowClose.is_trimmed_before(neighbor));
        }
    }

    #[test]
    fn untrimmed_pairs() {
        let (open, close) = (TrimClass::Open, TrimClass::Close);
        // An opening character followed by a closing one: the ink of both faces inward.
        assert!(!close.is_trimmed_after(open));
        assert!(!open.is_trimmed_before(close));
        // Next to ordinary characters.
        assert!(!open.is_trimmed_after(TrimClass::Other));
        assert!(!close.is_trimmed_before(TrimClass::Other));
        // Middle dots are never trimmed.
        for neighbor in [open, close, TrimClass::Middle, TrimClass::Other] {
            assert!(!TrimClass::Middle.is_trimmed_after(neighbor));
            assert!(!TrimClass::Middle.is_trimmed_before(neighbor));
        }
        // A closing character followed by an opening one trims the opening one only.
        assert!(open.is_trimmed_after(close));
        assert!(!close.is_trimmed_before(open));
    }

    /// The characters of `text` that are trimmed because of a neighbor, where each character has
    /// the style and bidi level at its index in `styles` and `bidi_levels`.
    fn trimmed(text: &str, styles: &[(TextSpacingTrim, &str)], bidi_levels: &[u8]) -> Vec<u32> {
        let styles: Vec<ResolvedStyle<()>> = (styles.iter())
            .map(|&(text_spacing_trim, language)| ResolvedStyle {
                text_spacing_trim,
                locale: Some(Language::parse(language).unwrap()),
                ..Default::default()
            })
            .collect();
        let char_style_indices: Vec<u16> = (0..).take(styles.len()).collect();
        let bidi_levels: Vec<_> = bidi_levels.iter().map(|&l| BidiLevel::new(l)).collect();
        let mut sites = TrimSites::default();
        sites.collect(
            text,
            &ResolveContext::default(),
            &styles,
            &char_style_indices,
            &bidi_levels,
        );
        sites.trims().map(|trim| trim.char_range.start).collect()
    }

    #[test]
    fn trim_sites_are_decided_by_the_style_of_the_trimmed_character() {
        let (normal, space_all) = (TextSpacingTrim::NORMAL, TextSpacingTrim::SPACE_ALL);
        let ja = (normal, "ja");
        assert_eq!(trimmed("「「」」", &[ja; 4], &[]), [1, 2]);
        // Only the style of the character that would be trimmed says whether it is.
        let styles = [ja, (space_all, "ja"), (space_all, "ja"), ja];
        assert_eq!(trimmed("「「」」", &styles, &[]), []);
        let styles = [(space_all, "ja"), ja, ja, (space_all, "ja")];
        assert_eq!(trimmed("「「」」", &styles, &[]), [1, 2]);

        // A dot is closing punctuation in Japanese, and a middle dot in Traditional Chinese.
        let zh_hant = (normal, "zh-Hant");
        assert_eq!(trimmed("。」", &[ja, zh_hant], &[]), [0]);
        assert_eq!(trimmed("。」", &[zh_hant, ja], &[]), []);
    }

    #[test]
    fn trim_sites_do_not_cross_direction_changes() {
        let ja = (TextSpacingTrim::NORMAL, "ja");
        assert_eq!(trimmed("「「」」", &[ja; 4], &[1; 4]), [1, 2]);
        assert_eq!(trimmed("「「」」", &[ja; 4], &[0, 1, 1, 0]), []);
        assert_eq!(trimmed("「「」」", &[ja; 4], &[0, 0, 1, 1]), [1, 2]);
    }
}
