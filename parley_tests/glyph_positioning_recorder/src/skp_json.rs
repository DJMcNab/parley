// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Turning Skia's own `skp_parser` draw-command dump into the shared golden schema.
//!
//! **Pure parsing only.** `skp_parser` itself now runs inside the container, invoked
//! over HTTP by [`crate::agent::AgentClient`] (`GET /skp/<name>/commands` and
//! `GET /skp/<name>/typeface`); this module never shells out to anything. See
//! `doc/glyph-positioning-recorder-agent.md`.
//!
//! **There is no transform stack.** Phase 0 found that `Save`/`SaveLayer`/`Restore`/
//! `Concat44` and friends do not appear at all — absolute coordinates are baked into
//! each `DrawTextBlob` origin — so that is taken as the contract and any transform
//! command is a hard error. See "Corrections to earlier phases" in
//! `doc/glyph-positioning-chrome-parity-phase4.md`.
//!
//! **Nothing here routes through [`serde_json::Value`].** Each command is read as a
//! `RawValue`, its `command` tag pulled out, and the concrete leaf shapes
//! (`Vec<f64>`, `Vec<[f64; 2]>`, `Vec<u32>`, …) deserialized from those same raw
//! slices. An internally-tagged enum derive would buffer through `Value`, which is
//! the documented `arbitrary_precision` hazard the parent plan requires avoiding —
//! this pattern sidesteps the feature interaction entirely rather than depending on
//! the feature not being enabled somewhere in the dependency graph.

use std::collections::BTreeMap;

use parley_glyph_positioning_cases::{
    GlyphOutput, GlyphOutputBuilder, Style, postscript_name_from_bytes, scan_for_valid_sfnts,
};
use serde_json::value::RawValue;

use crate::Result;

/// `SkPaintDefaults_TextSize`, which `MakeJsonFont`'s `store_scalar` omits from the
/// dump because it is the default — so a run at exactly 12px emits no `textSize`
/// field at all. 12px is inside the sampled size range, so absence must mean this,
/// not an error.
const DEFAULT_TEXT_SIZE: f32 = 12.0;

/// Commands that carry no glyph data and no transform, and so are skipped.
///
/// This list starts at what Phase 0 actually captured (`DrawPaint`, `DrawRect`) plus
/// the clip commands, and is extended as bring-up step B6's command inventory reports
/// more. Anything absent from it is fatal by design.
const IGNORED_COMMANDS: &[&str] = &[
    "DrawPaint",
    "DrawRect",
    "ClipRect",
    "ClipRRect",
    "ClipPath",
    "ClipRegion",
    "ClipShader",
];

/// A JSON object whose fields are kept as raw slices, so each can be deserialized
/// into its concrete shape on demand.
///
/// Duplicate keys resolve last-wins, which is exactly what is needed for `scaleX` and
/// `skewX`: Skia writes both under `DEBUGCANVAS_ATTRIBUTE_TEXTSCALEX`, so a non-zero
/// skew produces a duplicate key. v1 has no italic or synthetic oblique, so this is
/// recorded rather than handled.
type Object = BTreeMap<String, Box<RawValue>>;

/// One `skp_parser` command dump, reduced to the glyph data the comparison needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Capture {
    /// The single `data/N` key every run's typeface referenced.
    ///
    /// A capture referencing two or more is a font fallback and is refused, so this is
    /// always exactly one key.
    pub typeface_key: String,
    /// The captured glyphs, in emission order: command order, then run order, then
    /// index within the run.
    pub glyphs: Vec<CapturedGlyph>,
}

/// A single glyph read out of a `DrawTextBlob`, at its absolute document position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapturedGlyph {
    /// The glyph ID, in the run's typeface.
    pub id: u32,
    /// Absolute x position, in CSS px.
    pub x: f32,
    /// Absolute y position, in CSS px.
    pub y: f32,
    /// The run's font size, in CSS px.
    pub font_size: f32,
}

impl Capture {
    /// Parses one `skp_parser` command dump.
    pub fn parse(json: &str) -> Result<Self> {
        let dump: Object = serde_json::from_str(json)
            .map_err(|error| format!("the dump is not a JSON object: {error}"))?;
        let commands: Vec<Box<RawValue>> =
            serde_json::from_str(field(&dump, "commands", "the dump")?.get())
                .map_err(|error| format!("the dump's `commands` is not an array: {error}"))?;

        let mut glyphs = Vec::new();
        // Distinct `data/N` keys in first-appearance order; more than one means a font
        // fallback happened, which v1 has no way to model.
        let mut typeface_keys: Vec<String> = Vec::new();

        for (index, command) in commands.iter().enumerate() {
            let what = format!("commands[{index}]");
            let command = object(command, &what)?;
            let tag = string_field(&command, "command", &what)?;
            match tag.as_str() {
                "DrawTextBlob" => {
                    read_text_blob(&command, &what, &mut glyphs, &mut typeface_keys)?;
                }
                tag if IGNORED_COMMANDS.contains(&tag) => {}
                tag => {
                    return Err(format!(
                        "{what}: unhandled command `{tag}`. Every transform and save/restore \
                         command lands here deliberately: an unmodelled transform offsets every \
                         glyph uniformly, which is the hardest failure to diagnose, so it must \
                         never be silently skipped. If `{tag}` is inert, add it to \
                         `IGNORED_COMMANDS`; if it is a transform, the composition path Phase 4 \
                         deliberately did not write now has a real example to be written against"
                    )
                    .into());
                }
            }
        }

        match typeface_keys.len() {
            1 => Ok(Self {
                typeface_key: typeface_keys.remove(0),
                glyphs,
            }),
            0 => Err("the dump contains no `DrawTextBlob` runs, so nothing was captured".into()),
            _ => Err(format!(
                "the dump's runs reference {} distinct typefaces ({}); a font fallback occurred, \
                 which v1 cannot model — every run must use the single registered font",
                typeface_keys.len(),
                typeface_keys.join(", ")
            )
            .into()),
        }
    }

    /// Resolves this capture into the shared golden schema.
    ///
    /// `postscript_name` is the session's already-resolved typeface name (see
    /// [`typeface_postscript_name`]); it is resolved once per session rather than per
    /// capture, since it cannot change while the same font is registered.
    #[must_use]
    pub fn into_output(self, postscript_name: &str) -> GlyphOutput {
        let mut builder = GlyphOutputBuilder::default();
        for glyph in self.glyphs {
            let style = builder.style_index(Style {
                postscript_name: postscript_name.to_string(),
                font_size: glyph.font_size,
            });
            builder.push_glyph(glyph.id, glyph.x, glyph.y, style);
        }
        builder.build()
    }
}

/// Reads one `DrawTextBlob`'s runs, appending their glyphs to `glyphs` and recording
/// any newly-seen typeface key in `typeface_keys`.
fn read_text_blob(
    blob: &Object,
    what: &str,
    glyphs: &mut Vec<CapturedGlyph>,
    typeface_keys: &mut Vec<String>,
) -> Result<()> {
    // Absence means visible; we only know what `true` means, so `false` is refused
    // rather than skipped.
    if let Some(visible) = blob.get("visible") {
        let visible: bool = serde_json::from_str(visible.get())
            .map_err(|error| format!("{what}: `visible` is not a boolean: {error}"))?;
        if !visible {
            return Err(format!(
                "{what}: `visible` is false. Nothing is known to produce an invisible text blob, \
                 so this is refused rather than skipped — skipping would drop glyphs Parley still \
                 has, and report as a glyph-count mismatch"
            )
            .into());
        }
    }

    let blob_x = f64_field(blob, "x", what)?;
    let blob_y = f64_field(blob, "y", what)?;
    let runs: Vec<Box<RawValue>> = serde_json::from_str(field(blob, "runs", what)?.get())
        .map_err(|error| format!("{what}: `runs` is not an array: {error}"))?;

    for (index, run) in runs.iter().enumerate() {
        let what = format!("{what}.runs[{index}]");
        read_run(
            &object(run, &what)?,
            &what,
            blob_x,
            blob_y,
            glyphs,
            typeface_keys,
        )?;
    }
    Ok(())
}

/// Reads one run of a `DrawTextBlob`, whose origin is `(blob_x, blob_y)`.
fn read_run(
    run: &Object,
    what: &str,
    blob_x: f64,
    blob_y: f64,
    glyphs: &mut Vec<CapturedGlyph>,
    typeface_keys: &mut Vec<String>,
) -> Result<()> {
    let font = object(field(run, "font", what)?, what)?;
    let font_size = match font.get("textSize") {
        Some(raw) => f64_value(raw, &format!("{what}.font.textSize"))?,
        None => f64::from(DEFAULT_TEXT_SIZE),
    };

    let typeface = object(field(&font, "typeface", what)?, what)?;
    let key = string_field(&typeface, "data", &format!("{what}.font.typeface"))?;
    if !typeface_keys.contains(&key) {
        typeface_keys.push(key);
    }

    let ids: Vec<u32> = serde_json::from_str(field(run, "glyphs", what)?.get())
        .map_err(|error| format!("{what}: `glyphs` is not an array of glyph IDs: {error}"))?;

    let positions = field(run, "positions", what).map_err(|_| {
        format!(
            "{what}: no `positions`, i.e. Skia's kDefault positioning. Its per-glyph positions \
             are not recoverable from the dump, so this cannot be compared against Parley"
        )
    })?;

    // The JSON *shape* of `positions` is what selects the positioning mode. A flat
    // array is kHorizontal; an array of 2-element arrays is kFull. Trying the flat
    // shape first is unambiguous, because a kFull array's elements are arrays.
    let positions: Vec<(f64, f64)> =
        if let Ok(flat) = serde_json::from_str::<Vec<f64>>(positions.get()) {
            let coords: [f64; 2] = serde_json::from_str(field(run, "coords", what)?.get())
                .map_err(|error| format!("{what}: `coords` is not a 2-element array: {error}"))?;
            flat.into_iter()
                .map(|x| (blob_x + x, blob_y + coords[1]))
                .collect()
        } else {
            let full: Vec<[f64; 2]> = serde_json::from_str(positions.get()).map_err(|error| {
                format!(
                    "{what}: `positions` is neither a flat array (kHorizontal) nor an array of \
                     2-element arrays (kFull): {error}"
                )
            })?;
            full.into_iter()
                .map(|position| (blob_x + position[0], blob_y + position[1]))
                .collect()
        };

    if ids.len() != positions.len() {
        return Err(format!(
            "{what}: {} glyphs but {} positions",
            ids.len(),
            positions.len()
        )
        .into());
    }

    glyphs.extend(ids.into_iter().zip(positions).map(|(id, (x, y))| {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "`skp_parser` emits at most 6 significant figures, so nothing survives in \
                      f64 that f32 cannot hold; f64 is only used so the blob origin and the \
                      per-glyph offset are summed before narrowing"
        )]
        CapturedGlyph {
            id,
            x: x as f32,
            y: y as f32,
            font_size: font_size as f32,
        }
    }));
    Ok(())
}

/// Deserializes `raw` as a JSON object, tagging any failure with `what`.
fn object(raw: &RawValue, what: &str) -> Result<Object> {
    serde_json::from_str(raw.get())
        .map_err(|error| format!("{what}: expected a JSON object: {error}").into())
}

/// Looks up a required field, tagging its absence with `what`.
fn field<'a>(object: &'a Object, name: &str, what: &str) -> Result<&'a RawValue> {
    object
        .get(name)
        .map(Box::as_ref)
        .ok_or_else(|| format!("{what}: missing field `{name}`").into())
}

/// Deserializes `raw` as a number.
fn f64_value(raw: &RawValue, what: &str) -> Result<f64> {
    serde_json::from_str(raw.get())
        .map_err(|error| format!("{what}: expected a number: {error}").into())
}

/// Reads a required numeric field.
fn f64_field(object: &Object, name: &str, what: &str) -> Result<f64> {
    f64_value(field(object, name, what)?, &format!("{what}.{name}"))
}

/// Reads a required string field.
fn string_field(object: &Object, name: &str, what: &str) -> Result<String> {
    serde_json::from_str(field(object, name, what)?.get())
        .map_err(|error| format!("{what}.{name}: expected a string: {error}").into())
}

/// Resolves already-fetched typeface bytes (the body of `GET
/// /skp/<name>/typeface?key=<data-key>`) to the font's PostScript name.
///
/// The bytes Skia serializes wrap the font stream in its own container, so they are
/// scanned for a *validating* sfnt rather than trusted wholesale — a naive magic-byte
/// scan of a real capture yields nine false hits and one true one. Finding no sfnt at
/// all is itself the signal that Chrome fell back to a system font, which Skia may
/// serialize by descriptor with no embedded stream.
pub fn typeface_postscript_name(bytes: &[u8]) -> Result<String> {
    let offsets = scan_for_valid_sfnts(bytes);
    let [offset] = offsets[..] else {
        return Err(format!(
            "the typeface bytes contain {} valid sfnts in {} bytes, expected exactly 1. Zero \
             means Chrome fell back to a system font, which Skia serializes by descriptor with \
             no embedded font stream",
            offsets.len(),
            bytes.len()
        )
        .into());
    };
    postscript_name_from_bytes(&bytes[offset..], 0)
        .map_err(|error| format!("the typeface has no readable PostScript name: {error}").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real Phase 0 capture, taken with `--font-render-hinting=none` (the flag the
    /// production harness launches with).
    const PHASE0_LAYER_0: &str = include_str!("../tests/fixtures/phase0_layer_0.json");

    fn parse_ok(json: &str) -> Capture {
        Capture::parse(json).expect("fixture must parse")
    }

    fn parse_err(json: &str) -> String {
        Capture::parse(json)
            .expect_err("fixture must be refused")
            .to_string()
    }

    #[test]
    fn real_capture_glyph_positions() {
        let capture = parse_ok(PHASE0_LAYER_0);
        assert_eq!(capture.typeface_key, "data/0");
        // Four `DrawTextBlob`s of 16, 10, 1 and 5 glyphs.
        assert_eq!(capture.glyphs.len(), 32);

        // Blob 1: origin (0, 22), 24px, 16 glyphs, kHorizontal with `coords: [0, 0]`.
        let expected_first = [
            (44_u32, 0.0_f32),
            (73, 17.1094),
            (80, 29.8242),
            (80, 35.6484),
            (83, 41.4727),
            (4, 55.1602),
            (37, 61.1016),
            (58, 75.7383),
            (59, 91.0078),
            (37, 111.797),
            (4, 127.453),
            (91, 133.395),
            (83, 151.43),
            (86, 165.117),
            (80, 173.238),
            (72, 179.062),
        ];
        for (glyph, (id, x)) in capture.glyphs[..16].iter().zip(expected_first) {
            assert_eq!(
                *glyph,
                CapturedGlyph {
                    id,
                    x,
                    y: 22.0,
                    font_size: 24.0
                }
            );
        }

        // Blob 2: origin (192.609, 22), 13.37px, 10 glyphs.
        let expected_second = [
            (87_u32, 0.0_f64),
            (73, 8.38843),
            (71, 16.966),
            (83, 25.4588),
            (82, 34.5779),
            (72, 43.449),
            (4, 52.4832),
            (86, 57.2905),
            (89, 63.311),
            (82, 72.1756),
        ];
        for (glyph, (id, offset)) in capture.glyphs[16..26].iter().zip(expected_second) {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "mirrors the parser: the blob origin and the per-glyph offset are \
                          summed in f64 and narrowed once, so the expected value has to be \
                          computed the same way to be bit-identical"
            )]
            let x = (192.609_f64 + offset) as f32;
            assert_eq!(
                *glyph,
                CapturedGlyph {
                    id,
                    x,
                    y: 22.0,
                    font_size: 13.37
                }
            );
        }

        // Blob 3: origin (273.656, 22), 13.37px, a single glyph at offset 0.
        assert_eq!(
            capture.glyphs[26],
            CapturedGlyph {
                id: 4,
                x: 273.656,
                y: 22.0,
                font_size: 13.37
            }
        );

        // Blob 4: origin (0, 43) — the second line — 13.37px, 5 glyphs.
        let expected_fourth = [
            (88_u32, 0.0_f32),
            (83, 5.73341),
            (74, 14.8524),
            (89, 20.9904),
            (35, 29.855),
        ];
        for (glyph, (id, x)) in capture.glyphs[27..].iter().zip(expected_fourth) {
            assert_eq!(
                *glyph,
                CapturedGlyph {
                    id,
                    x,
                    y: 43.0,
                    font_size: 13.37
                }
            );
        }
    }

    #[test]
    fn real_capture_into_output() {
        let output = parse_ok(PHASE0_LAYER_0).into_output("Roboto-Regular");

        // Styles are deduplicated in first-appearance order: the 24px blob paints first.
        assert_eq!(
            output.styles,
            vec![
                Style {
                    postscript_name: "Roboto-Regular".to_string(),
                    font_size: 24.0
                },
                Style {
                    postscript_name: "Roboto-Regular".to_string(),
                    font_size: 13.37
                },
            ]
        );
        assert_eq!(output.glyphs.len(), 32);
        for glyph in &output.glyphs[..16] {
            assert_eq!(glyph.style, 0);
        }
        for glyph in &output.glyphs[16..] {
            assert_eq!(glyph.style, 1);
        }
    }

    #[test]
    fn two_typefaces_are_a_font_fallback() {
        let message = parse_err(include_str!("../tests/fixtures/reject_two_typefaces.json"));
        assert!(
            message.contains("2 distinct typefaces") && message.contains("font fallback"),
            "unhelpful message: {message}"
        );
    }

    #[test]
    fn a_transform_command_is_fatal() {
        let message = parse_err(include_str!("../tests/fixtures/reject_transform.json"));
        assert!(
            message.contains("Concat44") && message.contains("unhandled command"),
            "unhelpful message: {message}"
        );
    }

    #[test]
    fn default_positioning_is_unrecoverable() {
        let message = parse_err(include_str!(
            "../tests/fixtures/reject_default_positioning.json"
        ));
        assert!(
            message.contains("kDefault") && message.contains("not recoverable"),
            "unhelpful message: {message}"
        );
    }

    #[test]
    fn an_invisible_blob_is_fatal() {
        let message = parse_err(include_str!("../tests/fixtures/reject_invisible_blob.json"));
        assert!(
            message.contains("`visible` is false"),
            "unhelpful message: {message}"
        );
    }

    /// Still hand-written, and still the only coverage `kFull` has.
    ///
    /// Bring-up step B10 was meant to replace this with a real capture, and did not:
    /// it came back **kHorizontal**, not `kFull`, even for a run containing a
    /// standalone `Mn` combining mark (see [`combining_marks_are_horizontal`]). So
    /// `kFull`'s shape remains unobserved in practice, and this fixture stays derived
    /// from Skia's serialisation code rather than from Chrome.
    #[test]
    fn full_positioning_uses_both_components() {
        let capture = parse_ok(include_str!(
            "../tests/fixtures/provisional_full_positioning.json"
        ));
        assert_eq!(
            capture.glyphs,
            vec![
                CapturedGlyph {
                    id: 44,
                    x: 10.0,
                    y: 50.0,
                    font_size: 18.5
                },
                CapturedGlyph {
                    id: 780,
                    x: 18.5,
                    y: 46.75,
                    font_size: 18.5
                },
            ]
        );
    }

    /// Bring-up step B10's capture: `"z\u{0301}q\u{0303}w\u{0323}k\u{030F}"` at 22px,
    /// four bases each followed by a combining mark.
    ///
    /// Three things this pins down, all of which B10 existed to settle:
    ///
    /// - Positioning is **kHorizontal**, not `kFull` — so the whole run shares a
    ///   single `y`, taken from the blob origin plus `coords[1]`. A combining mark
    ///   does *not* get a per-glyph vertical position here, which is why
    ///   [`full_positioning_uses_both_components`] is still hand-written.
    /// - `HarfBuzz` precomposed three of the four pairs, but not `w` + U+0323: glyph
    ///   1223 survives as a standalone mark alongside its base, glyph 170.
    /// - That mark carries a **zero advance** — it sits at exactly its base's `x`,
    ///   not one advance past it.
    #[test]
    fn combining_marks_are_horizontal() {
        let capture = parse_ok(include_str!("../tests/fixtures/b10_combining_marks.json"));
        let expected = [
            (806_u32, 0.0_f32),
            (85, 10.7314),
            (170, 23.2354),
            (1223, 23.2354),
            (79, 39.7676),
            (172, 50.918),
        ];
        assert_eq!(
            capture.glyphs,
            expected
                .iter()
                .map(|&(id, x)| CapturedGlyph {
                    id,
                    x,
                    // One shared `y` for every glyph: that is what kHorizontal means.
                    y: 20.0,
                    font_size: 22.0,
                })
                .collect::<Vec<_>>()
        );

        let (base, mark) = (capture.glyphs[2], capture.glyphs[3]);
        assert_eq!(
            mark.x, base.x,
            "the standalone combining mark must have zero advance, i.e. sit at its base's x"
        );
    }

    /// Bring-up step B11's capture: `"twelve pixels"` at exactly 12px.
    ///
    /// The run's `font` object carries no `textSize` at all — `MakeJsonFont` omits
    /// fields equal to their default, and `SkPaintDefaults_TextSize` is 12 — so this
    /// is the real-data confirmation that absence must mean 12.0 rather than be an
    /// error. 12px is inside the sampled size range, so this is reachable by ordinary
    /// generated cases, not just by a handwritten one.
    #[test]
    fn absent_text_size_defaults_to_twelve() {
        let fixture = include_str!("../tests/fixtures/b11_twelve_px.json");
        assert!(
            !fixture.contains("textSize"),
            "this fixture is only meaningful while it has no `textSize` field"
        );

        let capture = parse_ok(fixture);
        let expected = [
            (88_u32, 0.0_f32),
            (91, 3.91992),
            (73, 12.9375),
            (80, 19.2949),
            (90, 22.207),
            (73, 27.9434),
            (4, 34.3008),
            (84, 37.2715),
            (77, 44.0039),
            (92, 46.916),
            (73, 52.7461),
            (80, 59.1035),
            (87, 62.0156),
        ];
        assert_eq!(
            capture.glyphs,
            expected
                .iter()
                .map(|&(id, x)| CapturedGlyph {
                    id,
                    x,
                    y: 11.0,
                    font_size: 12.0,
                })
                .collect::<Vec<_>>()
        );

        assert_eq!(
            parse_ok(fixture).into_output("Roboto-Regular").styles,
            vec![Style {
                postscript_name: "Roboto-Regular".to_string(),
                font_size: 12.0
            }]
        );
    }
}
