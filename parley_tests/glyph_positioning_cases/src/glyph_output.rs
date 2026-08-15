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
/// **There is no line concept**: Chrome exposes a true per-glyph `y`, so no line
/// grouping needs inferring, and the schema survives future vertical-align work where
/// glyphs on one line may sit on different baselines. Line grouping may be re-derived
/// best-effort in failure *reporting*, never in this schema.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOutput {
    /// The distinct styles referenced by [`Self::glyphs`], in first-appearance order.
    pub styles: Vec<Style>,
    /// The positioned glyphs, in the order they were produced.
    pub glyphs: Vec<PositionedGlyph>,
}

/// A style referenced by one or more [`PositionedGlyph`]s.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// The selected font's PostScript name (`name` table entry, ID 6).
    pub postscript_name: String,
    /// The (already Chromium-quantized) font size, in CSS px.
    pub font_size: f32,
}

/// A single positioned glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// The glyph ID, in the selected font.
    pub id: u32,
    /// Absolute x position, in CSS px.
    pub x: f32,
    /// Absolute y position, in CSS px.
    pub y: f32,
    /// Index into the owning [`GlyphOutput`]'s [`GlyphOutput::styles`].
    pub style: u16,
}

/// Incrementally builds a [`GlyphOutput`], deduplicating styles in first-appearance
/// order.
#[derive(Clone, Debug, Default)]
pub struct GlyphOutputBuilder {
    styles: Vec<Style>,
    glyphs: Vec<PositionedGlyph>,
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

    /// Appends a positioned glyph.
    pub fn push_glyph(&mut self, id: u32, x: f32, y: f32, style: u16) {
        self.glyphs.push(PositionedGlyph { id, x, y, style });
    }

    /// Finishes building, returning the completed [`GlyphOutput`].
    #[must_use]
    pub fn build(self) -> GlyphOutput {
        GlyphOutput {
            styles: self.styles,
            glyphs: self.glyphs,
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
        writeln!(out, "glyphs {}", self.output.glyphs.len()).unwrap();
        for glyph in &self.output.glyphs {
            writeln!(
                out,
                "glyph {} {} {} {}",
                glyph.id, glyph.x, glyph.y, glyph.style
            )
            .unwrap();
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

        let glyph_count: usize = parse_tagged_line(lines.next(), "glyphs")?;
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
            let style = parse_field(&mut fields, "glyph.style")?;
            glyphs.push(PositionedGlyph { id, x, y, style });
        }

        Ok(Self {
            case: Case { seed, runs, width },
            note,
            output: GlyphOutput { styles, glyphs },
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
