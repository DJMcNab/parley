// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! The golden output schema, a builder for constructing it, and a hand-rolled compact
//! text format for storing a [`Case`] alongside its recorded output.
//!
//! See "Golden output schema" and "File format" in
//! `doc/glyph-positioning-chrome-parity-phase1.md`. No `serde` dependency is taken —
//! see that section for why.

use std::fmt::Write as _;

use crate::generate::{Case, Run};

/// The deduplicated glyph output of laying out a [`Case`], from either Parley or
/// Chrome.
///
/// **Positions are stored per fragment, not absolutely.** A *fragment* is the unit
/// Blink positions as a whole — one span's glyphs on one line — and both sides record
/// the fragment's origin plus each glyph's offset from it. Two things depend on that
/// split:
///
/// - Blink snaps every fragment origin after the first onto `LayoutUnit`'s 1/64 px
///   grid, so the origin is where the two sides can legitimately differ while every
///   glyph within a fragment still has to agree exactly.
/// - `skp_parser` serialises the fragment origin and the glyph's offset as two
///   *separate* 6-significant-figure numbers, so the error floor on an absolute
///   position is the sum of their two half-ULPs. Storing only the sum throws away the
///   information needed to compute that floor.
///
/// **There is no line concept**: Chrome exposes a true per-glyph `y`, so no line
/// grouping needs inferring, and the schema survives future vertical-align work where
/// glyphs on one line may sit on different baselines. Line grouping may be re-derived
/// best-effort in failure *reporting*, never in this schema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOutput {
    /// The distinct styles referenced by [`Self::fragments`], in first-appearance
    /// order.
    pub styles: Vec<Style>,
    /// The fragments, in the order they were produced.
    pub fragments: Vec<Fragment>,
}

impl GlyphOutput {
    /// The glyphs of every fragment, at absolute positions, in emission order.
    ///
    /// This is the flat view the comparison pairs on and the report generator draws;
    /// the stored form is [`Self::fragments`], since the absolute position alone can't
    /// express either the snapping or the serialisation floor (see the type docs).
    #[must_use]
    pub fn glyphs(&self) -> Vec<PositionedGlyph> {
        self.fragments
            .iter()
            .flat_map(|fragment| {
                fragment.glyphs.iter().map(move |glyph| PositionedGlyph {
                    id: glyph.id,
                    x: fragment.origin_x + f64::from(glyph.x),
                    y: fragment.origin_y + f64::from(glyph.y),
                    style: fragment.style,
                })
            })
            .collect()
    }

    /// How many glyphs this output holds in total.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.fragments
            .iter()
            .map(|fragment| fragment.glyphs.len())
            .sum()
    }

    /// The number of glyphs in each fragment, in order — this output's *shape*, which
    /// the comparison checks before it looks at any position.
    #[must_use]
    pub fn shape(&self) -> Vec<usize> {
        self.fragments
            .iter()
            .map(|fragment| fragment.glyphs.len())
            .collect()
    }
}

/// A style referenced by one or more [`Fragment`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// The selected font's PostScript name (`name` table entry, ID 6).
    pub postscript_name: String,
    /// The (already Chromium-quantized) font size, in CSS px.
    pub font_size: f32,
}

/// One span's glyphs on one line: the unit Blink positions as a whole.
///
/// Blink lays a line out by placing each fragment at the previous one's ceil-snapped
/// end (`ShapeResult::SnappedWidth()`, a `LayoutUnit::FromFloatCeil`), so
/// [`Self::origin_x`] is where the 1/64 px grid enters the comparison. Within a
/// fragment, glyph offsets are unrounded on both sides.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    /// The fragment's inline origin, in CSS px. Every glyph's [`LocalGlyph::x`] is
    /// relative to this.
    ///
    /// f64, unlike the glyph offsets, because an origin is an *accumulator*: Blink
    /// sums snapped fragment widths across a line, Parley sums advances in f64 for the
    /// same reason, and Chrome's is a decimal this crate must not re-round. Narrowing
    /// it to f32 costs half an f32 ULP — around 8e-6 px at the widths the corpus
    /// reaches, which is a sixth of the serialisation floor and enough on its own to
    /// push a matching case over it. A glyph offset is not an accumulator: it is one
    /// shaped value, f32-native on both sides, so it stays f32.
    pub origin_x: f64,
    /// The fragment's baseline, in CSS px. Every glyph's [`LocalGlyph::y`] is relative
    /// to this. f64 for the same reason as [`Self::origin_x`].
    pub origin_y: f64,
    /// Index into the owning [`GlyphOutput`]'s [`GlyphOutput::styles`].
    pub style: u16,
    /// The fragment's glyphs, in emission order.
    pub glyphs: Vec<LocalGlyph>,
}

/// A glyph positioned relative to its owning [`Fragment`]'s origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalGlyph {
    /// The glyph ID, in the selected font.
    pub id: u32,
    /// Offset from [`Fragment::origin_x`], in CSS px.
    pub x: f32,
    /// Offset from [`Fragment::origin_y`], in CSS px.
    pub y: f32,
}

/// A single glyph at an absolute position — the derived view [`GlyphOutput::glyphs`]
/// produces, never the stored one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// The glyph ID, in the selected font.
    pub id: u32,
    /// Absolute x position, in CSS px. f64, so summing the origin and the offset does
    /// not re-round what [`Fragment::origin_x`] is f64 to preserve.
    pub x: f64,
    /// Absolute y position, in CSS px. f64, as [`Self::x`].
    pub y: f64,
    /// Index into the owning [`GlyphOutput`]'s [`GlyphOutput::styles`].
    pub style: u16,
}

/// Incrementally builds a [`GlyphOutput`], deduplicating styles in first-appearance
/// order.
#[derive(Clone, Debug, Default)]
pub struct GlyphOutputBuilder {
    styles: Vec<Style>,
    fragments: Vec<Fragment>,
}

impl GlyphOutputBuilder {
    /// Returns the index of `style` in the (deduplicated) style table, inserting it if
    /// this is the first time it's been seen.
    pub fn style_index(&mut self, style: Style) -> u16 {
        if let Some(index) = self.styles.iter().position(|existing| *existing == style) {
            u16::try_from(index).expect("style table stays well under u16::MAX entries")
        } else {
            self.styles.push(style);
            u16::try_from(self.styles.len() - 1)
                .expect("style table stays well under u16::MAX entries")
        }
    }

    /// Starts a new fragment. Subsequent [`Self::push_glyph`] calls append to it.
    pub fn begin_fragment(&mut self, origin_x: f64, origin_y: f64, style: u16) {
        self.fragments.push(Fragment {
            origin_x,
            origin_y,
            style,
            glyphs: Vec::new(),
        });
    }

    /// Appends a glyph, at an offset from the current fragment's origin.
    ///
    /// # Panics
    ///
    /// If no fragment has been started with [`Self::begin_fragment`].
    pub fn push_glyph(&mut self, id: u32, x: f32, y: f32) {
        self.fragments
            .last_mut()
            .expect("begin_fragment must be called before push_glyph")
            .glyphs
            .push(LocalGlyph { id, x, y });
    }

    /// Finishes building, returning the completed [`GlyphOutput`].
    ///
    /// Fragments that ended up with no glyphs are dropped: an empty fragment has no
    /// position to compare and would only make the two sides' shapes disagree for a
    /// reason neither side can act on.
    #[must_use]
    pub fn build(mut self) -> GlyphOutput {
        self.fragments
            .retain(|fragment| !fragment.glyphs.is_empty());
        GlyphOutput {
            styles: self.styles,
            fragments: self.fragments,
        }
    }
}

/// A golden test fixture: the full [`Case`] that produced some output, alongside that
/// output.
///
/// Every golden — handwritten, promoted-regression, or generated — stores the full
/// `Case`, not just its seed; the seed is provenance only. See "Golden CI set" in
/// `doc/glyph-positioning-chrome-parity.md` for why.
#[derive(Clone, Debug, PartialEq)]
pub struct Golden {
    /// The case that was laid out.
    pub case: Case,
    /// Why this golden exists, for a case authored by hand rather than generated.
    ///
    /// The format has no comments, so this is the only place a handwritten case can
    /// say what it is for. `regenerate_goldens` preserves it when it re-records a file.
    pub note: Option<String>,
    /// The recorded output (Chrome's, for a checked-in golden; Parley's, when
    /// comparing).
    pub output: GlyphOutput,
}

/// An error parsing a [`Golden`] from its text format.
#[derive(Debug)]
pub struct ParseGoldenError {
    message: String,
}

impl std::fmt::Display for ParseGoldenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseGoldenError {}

fn parse_error(message: impl Into<String>) -> ParseGoldenError {
    ParseGoldenError {
        message: message.into(),
    }
}

impl Golden {
    /// Serializes this golden to the hand-rolled compact text format described in
    /// "File format" in the Phase 1 doc.
    ///
    /// This is a line-oriented format. Header lines are whitespace-separated fields;
    /// a run's or style's text/name payload is never escaped, since the sampling
    /// alphabet excludes every control character, so it's written verbatim on its own
    /// line (never trimmed when read back).
    #[must_use]
    pub fn write(&self) -> String {
        let mut out = String::new();
        writeln!(out, "seed {}", self.case.seed).unwrap();
        if let Some(note) = &self.note {
            writeln!(out, "note {note}").unwrap();
        }
        writeln!(out, "width {}", self.case.width).unwrap();
        writeln!(out, "runs {}", self.case.runs.len()).unwrap();
        for run in &self.case.runs {
            writeln!(
                out,
                "run {} {} {} {}",
                run.font_size, run.letter_spacing, run.word_spacing, run.line_height
            )
            .unwrap();
            writeln!(out, "{}", run.text).unwrap();
        }
        writeln!(out, "styles {}", self.output.styles.len()).unwrap();
        for style in &self.output.styles {
            writeln!(out, "style {} {}", style.font_size, style.postscript_name).unwrap();
        }
        writeln!(out, "fragments {}", self.output.fragments.len()).unwrap();
        for fragment in &self.output.fragments {
            writeln!(
                out,
                "fragment {} {} {} {}",
                fragment.origin_x,
                fragment.origin_y,
                fragment.style,
                fragment.glyphs.len()
            )
            .unwrap();
            for glyph in &fragment.glyphs {
                writeln!(out, "glyph {} {} {}", glyph.id, glyph.x, glyph.y).unwrap();
            }
        }
        out
    }

    /// Parses a [`Golden`] written by [`Self::write`].
    pub fn parse(text: &str) -> Result<Self, ParseGoldenError> {
        let mut lines = text.lines().peekable();

        let seed = parse_tagged_line(lines.next(), "seed")?;
        let note = lines
            .peek()
            .and_then(|line| line.strip_prefix("note "))
            .map(ToString::to_string);
        if note.is_some() {
            lines.next();
        }
        let width = parse_tagged_line(lines.next(), "width")?;

        let run_count: usize = parse_tagged_line(lines.next(), "runs")?;
        let mut runs = Vec::with_capacity(run_count);
        for _ in 0..run_count {
            let header = lines
                .next()
                .ok_or_else(|| parse_error("unexpected end of input reading a `run` header"))?;
            let mut fields = header
                .strip_prefix("run ")
                .ok_or_else(|| parse_error(format!("expected `run ...`, got {header:?}")))?
                .split(' ');
            let font_size = parse_field(&mut fields, "run.font_size")?;
            let letter_spacing = parse_field(&mut fields, "run.letter_spacing")?;
            let word_spacing = parse_field(&mut fields, "run.word_spacing")?;
            let line_height = parse_field(&mut fields, "run.line_height")?;
            let text = lines
                .next()
                .ok_or_else(|| parse_error("unexpected end of input reading a run's text"))?
                .to_string();
            runs.push(Run {
                text,
                font_size,
                letter_spacing,
                word_spacing,
                line_height,
            });
        }

        let style_count: usize = parse_tagged_line(lines.next(), "styles")?;
        let mut styles = Vec::with_capacity(style_count);
        for _ in 0..style_count {
            let line = lines
                .next()
                .ok_or_else(|| parse_error("unexpected end of input reading a `style` line"))?;
            let rest = line
                .strip_prefix("style ")
                .ok_or_else(|| parse_error(format!("expected `style ...`, got {line:?}")))?;
            let (font_size_str, postscript_name) = rest.split_once(' ').ok_or_else(|| {
                parse_error(format!("expected `style <size> <name>`, got {line:?}"))
            })?;
            let font_size = font_size_str
                .parse()
                .map_err(|_| parse_error(format!("invalid style.font_size in {line:?}")))?;
            styles.push(Style {
                postscript_name: postscript_name.to_string(),
                font_size,
            });
        }

        let fragment_count: usize = parse_tagged_line(lines.next(), "fragments")?;
        let mut fragments = Vec::with_capacity(fragment_count);
        for _ in 0..fragment_count {
            let header = lines.next().ok_or_else(|| {
                parse_error("unexpected end of input reading a `fragment` header")
            })?;
            let mut fields = header
                .strip_prefix("fragment ")
                .ok_or_else(|| parse_error(format!("expected `fragment ...`, got {header:?}")))?
                .split(' ');
            let origin_x = parse_field(&mut fields, "fragment.origin_x")?;
            let origin_y = parse_field(&mut fields, "fragment.origin_y")?;
            let style = parse_field(&mut fields, "fragment.style")?;
            let glyph_count: usize = parse_field(&mut fields, "fragment.glyph_count")?;

            let mut glyphs = Vec::with_capacity(glyph_count);
            for _ in 0..glyph_count {
                let line = lines
                    .next()
                    .ok_or_else(|| parse_error("unexpected end of input reading a `glyph` line"))?;
                let mut fields = line
                    .strip_prefix("glyph ")
                    .ok_or_else(|| parse_error(format!("expected `glyph ...`, got {line:?}")))?
                    .split(' ');
                let id = parse_field(&mut fields, "glyph.id")?;
                let x = parse_field(&mut fields, "glyph.x")?;
                let y = parse_field(&mut fields, "glyph.y")?;
                glyphs.push(LocalGlyph { id, x, y });
            }
            fragments.push(Fragment {
                origin_x,
                origin_y,
                style,
                glyphs,
            });
        }

        Ok(Self {
            case: Case { seed, runs, width },
            note,
            output: GlyphOutput { styles, fragments },
        })
    }
}

/// Parses a `<tag> <value>` line, requiring the tag to match `tag`.
fn parse_tagged_line<T: std::str::FromStr>(
    line: Option<&str>,
    tag: &str,
) -> Result<T, ParseGoldenError> {
    let line =
        line.ok_or_else(|| parse_error(format!("unexpected end of input, expected `{tag} ...`")))?;
    let value = line
        .strip_prefix(tag)
        .and_then(|rest| rest.strip_prefix(' '))
        .ok_or_else(|| parse_error(format!("expected `{tag} ...`, got {line:?}")))?;
    value
        .parse()
        .map_err(|_| parse_error(format!("invalid value for `{tag}`: {value:?}")))
}

/// Parses the next whitespace-separated field from `fields`, tagging any error with
/// `field_name`.
fn parse_field<T: std::str::FromStr>(
    fields: &mut std::str::Split<'_, char>,
    field_name: &str,
) -> Result<T, ParseGoldenError> {
    fields
        .next()
        .ok_or_else(|| parse_error(format!("missing field `{field_name}`")))?
        .parse()
        .map_err(|_| parse_error(format!("invalid value for field `{field_name}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::Case;

    #[test]
    fn round_trips_a_line_height() {
        let original = Golden {
            case: Case {
                seed: 42,
                runs: vec![Run {
                    text: "hi".to_string(),
                    font_size: 16.0,
                    letter_spacing: 0.0,
                    word_spacing: 0.0,
                    line_height: 20.5,
                }],
                width: 100.0,
            },
            note: None,
            output: GlyphOutput::default(),
        };
        assert_eq!(Golden::parse(&original.write()).unwrap(), original);
    }
}
