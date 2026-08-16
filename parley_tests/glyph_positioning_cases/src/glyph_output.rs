// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::fmt::Write as _;

use crate::generate::{Case, Run};

/// Positioned glyphs emitted by a layout engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphOutput {
    /// Styles referenced by fragments.
    pub styles: Vec<Style>,
    /// Glyph fragments in emission order.
    pub fragments: Vec<Fragment>,
}

impl GlyphOutput {
    /// Returns all glyphs with fragment origins applied.
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

    /// Returns the total number of glyphs.
    #[must_use]
    pub fn glyph_count(&self) -> usize {
        self.fragments
            .iter()
            .map(|fragment| fragment.glyphs.len())
            .sum()
    }

    /// Returns the glyph count of each fragment.
    #[must_use]
    pub fn shape(&self) -> Vec<usize> {
        self.fragments
            .iter()
            .map(|fragment| fragment.glyphs.len())
            .collect()
    }
}

/// Font properties recorded for a fragment.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Font PostScript name.
    pub postscript_name: String,
    /// Font size in CSS pixels.
    pub font_size: f32,
}

/// A run of glyphs sharing an origin and style.
#[derive(Clone, Debug, PartialEq)]
pub struct Fragment {
    /// Horizontal origin in CSS pixels.
    pub origin_x: f64,
    /// Vertical origin in CSS pixels.
    pub origin_y: f64,
    /// Index into [`GlyphOutput::styles`].
    pub style: u16,
    /// Glyphs positioned relative to this fragment's origin.
    pub glyphs: Vec<LocalGlyph>,
}

/// A glyph positioned relative to its fragment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalGlyph {
    /// Font glyph identifier.
    pub id: u32,
    /// Horizontal offset in CSS pixels.
    pub x: f32,
    /// Vertical offset in CSS pixels.
    pub y: f32,
}

/// A glyph with an absolute position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PositionedGlyph {
    /// Font glyph identifier.
    pub id: u32,
    /// Horizontal position in CSS pixels.
    pub x: f64,
    /// Vertical position in CSS pixels.
    pub y: f64,
    /// Index into the output's style table.
    pub style: u16,
}

/// Builds a [`GlyphOutput`] while deduplicating styles.
#[derive(Clone, Debug, Default)]
pub struct GlyphOutputBuilder {
    styles: Vec<Style>,
    fragments: Vec<Fragment>,
}

impl GlyphOutputBuilder {
    /// Adds or finds `style` and returns its table index.
    pub fn style_index(&mut self, style: Style) -> u16 {
        if let Some(index) = self.styles.iter().position(|existing| *existing == style) {
            u16::try_from(index).expect("style table stays well under u16::MAX entries")
        } else {
            self.styles.push(style);
            u16::try_from(self.styles.len() - 1)
                .expect("style table stays well under u16::MAX entries")
        }
    }

    /// Starts a fragment receiving subsequent glyphs.
    pub fn begin_fragment(&mut self, origin_x: f64, origin_y: f64, style: u16) {
        self.fragments.push(Fragment {
            origin_x,
            origin_y,
            style,
            glyphs: Vec::new(),
        });
    }

    /// Adds a glyph to the current fragment.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::begin_fragment`] has not been called.
    pub fn push_glyph(&mut self, id: u32, x: f32, y: f32) {
        self.fragments
            .last_mut()
            .expect("begin_fragment must be called before push_glyph")
            .glyphs
            .push(LocalGlyph { id, x, y });
    }

    /// Finishes the output, discarding empty fragments.
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

/// A test case and its recorded reference output.
#[derive(Clone, Debug, PartialEq)]
pub struct Golden {
    /// Input case.
    pub case: Case,
    /// Optional provenance or known-failure note.
    pub note: Option<String>,
    /// Recorded Chromium output.
    pub output: GlyphOutput,
}

/// An error parsing a golden file.
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
    /// Serializes this golden file.
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

    /// Parses a golden file.
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
