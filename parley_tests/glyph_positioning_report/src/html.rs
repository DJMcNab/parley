// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Small helpers shared by the page and index generators.

/// The tinting filters the overlay uses, inlined into every page that overlays the two
/// sides.
///
/// The two rasters are black-on-transparent, so the overlay has to colour them. The
/// obvious way — a coloured element with the raster as a CSS `mask-image` — is not
/// usable here: mask images are CORS-restricted, and on a `file://` page every sibling
/// file is a foreign origin, so the index's thumbnails would silently come out blank.
/// An `<img>` has no such restriction, and a same-document SVG filter recolours it
/// while preserving its alpha, so page and index can share one implementation.
pub(crate) const TINT_FILTERS: &str = concat!(
    "<svg class=\"filters\" width=\"0\" height=\"0\" aria-hidden=\"true\">",
    "<filter id=\"tint-chrome\"><feColorMatrix type=\"matrix\" values=\"",
    "0 0 0 0 0.85  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0\"/></filter>",
    "<filter id=\"tint-parley\"><feColorMatrix type=\"matrix\" values=\"",
    "0 0 0 0 0  0 0 0 0 0.15  0 0 0 0 0.85  0 0 0 1 0\"/></filter>",
    "</svg>",
);

/// Escapes `text` for use in HTML text content or a double-quoted attribute.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Encodes `bytes` as standard base64, for `data:` URIs.
///
/// Hand-rolled rather than taken as a dependency: this is the only base64 in the crate,
/// and it is write-only.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0_u32, |acc, (index, &byte)| {
            acc | (u32::from(byte) << (16 - 8 * index))
        });
        for index in 0..chunk.len() + 1 {
            out.push(char::from(
                ALPHABET[((bits >> (18 - 6 * index)) & 0x3f) as usize],
            ));
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
}

/// The pastel hue for a cluster, spread by the golden angle so neighbouring clusters
/// never share a colour.
pub(crate) fn cluster_hue(index: usize) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "cluster counts are in the hundreds at most"
    )]
    let index = index as f32;
    (index * 137.508) % 360.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        // The classic RFC 4648 test vectors, which exercise all three padding cases.
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_round_trips_high_bytes() {
        // 0xfb 0xff 0xbf exercises every bit position of the 6-bit regrouping.
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
