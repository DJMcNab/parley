// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Typeface identity: reading a font's PostScript name, and validating that a byte
//! range actually parses as a well-formed sfnt before trusting it.
//!
//! See "Typeface identity" in `doc/glyph-positioning-chrome-parity-phase1.md`.

use read_fonts::{FileRef, FontRef, ReadError, TableProvider, types::NameId};

/// An error reading a font's PostScript name.
#[derive(Debug)]
pub enum TypefaceError {
    /// The font data failed to parse.
    Read(ReadError),
    /// The font parsed, but has no PostScript name (`name` table entry with ID 6).
    MissingPostscriptName,
}

impl std::fmt::Display for TypefaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "font data failed to parse: {error}"),
            Self::MissingPostscriptName => {
                f.write_str("font has no PostScript name (`name` table entry ID 6)")
            }
        }
    }
}

impl std::error::Error for TypefaceError {}

/// Reads the PostScript name (`name` table entry, ID 6) from a single sfnt font.
///
/// `collection_index` is the font's index within a TrueType collection, or `0` for a
/// standalone font — this is meant to be called directly with a Parley `FontData`'s
/// `data` and `index` fields.
pub fn postscript_name_from_bytes(
    font_data: &[u8],
    collection_index: u32,
) -> Result<String, TypefaceError> {
    let font = FontRef::from_index(font_data, collection_index).map_err(TypefaceError::Read)?;
    postscript_name(&font)
}

/// Reads the PostScript name (`name` table entry, ID 6) from an already-parsed font.
fn postscript_name(font: &FontRef<'_>) -> Result<String, TypefaceError> {
    let name = font.name().map_err(TypefaceError::Read)?;
    let string_data = name.string_data();
    name.name_record()
        .iter()
        .filter(|record| record.name_id() == NameId::POSTSCRIPT_NAME)
        .find_map(|record| record.string(string_data).ok())
        .map(|s| s.to_string())
        .ok_or(TypefaceError::MissingPostscriptName)
}

/// Scans `data` for byte offsets at which a well-formed sfnt begins: a font (or
/// collection thereof) whose table directory parses cleanly and which has both a
/// `name` and a `cmap` table.
///
/// A naive magic-byte scan over-matches — Phase 0 found 9 false hits and 1 true one in
/// a real `skp_parser`-serialized typeface blob — so this additionally requires the
/// candidate to actually parse as a font with those two tables present.
#[must_use]
pub fn scan_for_valid_sfnts(data: &[u8]) -> Vec<usize> {
    /// Magic bytes at the start of a well-formed sfnt: version 1.0, `OTTO`
    /// (CFF-flavored OpenType), and `ttcf` (TrueType collection).
    const MAGICS: [[u8; 4]; 3] = [[0x00, 0x01, 0x00, 0x00], *b"OTTO", *b"ttcf"];

    (0..data.len())
        .filter(|&offset| {
            data.get(offset..offset + 4)
                .is_some_and(|head| MAGICS.contains(&head.try_into().unwrap()))
                && is_valid_sfnt(&data[offset..])
        })
        .collect()
}

/// Returns whether `candidate` parses as a font (or collection) with both a `name` and
/// a `cmap` table.
fn is_valid_sfnt(candidate: &[u8]) -> bool {
    match FileRef::new(candidate) {
        Ok(FileRef::Font(font)) => has_name_and_cmap(&font),
        Ok(FileRef::Collection(collection)) => collection
            .iter()
            .any(|font| font.is_ok_and(|font| has_name_and_cmap(&font))),
        Err(_) => false,
    }
}

/// Returns whether `font` has both a `name` and a `cmap` table.
fn has_name_and_cmap(font: &FontRef<'_>) -> bool {
    font.name().is_ok() && font.cmap().is_ok()
}
