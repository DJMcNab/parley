// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use read_fonts::{FileRef, FontRef, ReadError, TableProvider, types::NameId};

/// An error reading a font's PostScript name.
#[derive(Debug)]
pub enum TypefaceError {
    /// The font could not be parsed.
    Read(ReadError),
    /// The font has no PostScript name record.
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

/// Reads a font's PostScript name.
pub fn postscript_name_from_bytes(
    font_data: &[u8],
    collection_index: u32,
) -> Result<String, TypefaceError> {
    let font = FontRef::from_index(font_data, collection_index).map_err(TypefaceError::Read)?;
    postscript_name(&font)
}

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

#[must_use]
/// Finds offsets at which a font with readable `name` and `cmap` tables begins.
pub fn scan_for_valid_sfnts(data: &[u8]) -> Vec<usize> {
    // TrueType 1.0, CFF OpenType, and TrueType collection headers.
    const MAGICS: [[u8; 4]; 3] = [[0x00, 0x01, 0x00, 0x00], *b"OTTO", *b"ttcf"];

    (0..data.len())
        .filter(|&offset| {
            data.get(offset..offset + 4)
                .is_some_and(|head| MAGICS.contains(&head.try_into().unwrap()))
                && is_valid_sfnt(&data[offset..])
        })
        .collect()
}

fn is_valid_sfnt(candidate: &[u8]) -> bool {
    match FileRef::new(candidate) {
        Ok(FileRef::Font(font)) => has_name_and_cmap(&font),
        Ok(FileRef::Collection(collection)) => collection
            .iter()
            .any(|font| font.is_ok_and(|font| has_name_and_cmap(&font))),
        Err(_) => false,
    }
}

fn has_name_and_cmap(font: &FontRef<'_>) -> bool {
    font.name().is_ok() && font.cmap().is_ok()
}
