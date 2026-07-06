// Kashida investigation: probe HarfRust shaping of Arabic, focusing on the
// SAFE_TO_INSERT_TATWEEL glyph flag and whether naive post-shaping tatweel
// insertion matches a full re-shape.
//
// Usage:
//   cargo run -- [path-to-font.ttf]
// Defaults to the OFL-licensed Noto Kufi Arabic already vendored in the repo
// (used by-path only for a local experiment; NOT copied/redistributed).

use harfrust::{BufferFlags, Direction, FontRef, ShaperData, ShapeOptions, UnicodeBuffer, script};
use skrifa::MetadataProvider;
use skrifa::prelude::Size;

const DEFAULT_FONT: &str =
    "../../../parley_dev/assets/fonts/noto_fonts/NotoKufiArabic-Regular.otf";

const TATWEEL: char = '\u{0640}';

fn flag_str(f: harfrust::GlyphFlags) -> String {
    let mut s = Vec::new();
    if f.is_unsafe_to_break() {
        s.push("UNSAFE_TO_BREAK");
    }
    if f.is_unsafe_to_concat() {
        s.push("UNSAFE_TO_CONCAT");
    }
    if f.is_safe_to_insert_tatweel() {
        s.push("SAFE_TO_INSERT_TATWEEL");
    }
    if s.is_empty() {
        "-".to_string()
    } else {
        s.join("|")
    }
}

fn shape_and_print(font_bytes: &[u8], label: &str, text: &str, produce_tatweel: bool) {
    let font_ref = FontRef::from_index(font_bytes, 0).unwrap();
    let data = ShaperData::new(&font_ref);
    let shaper = data.shaper(&font_ref).build();

    let mut buffer = UnicodeBuffer::new();
    for (i, ch) in text.chars().enumerate() {
        buffer.add(ch, i as u32);
    }
    buffer.set_direction(Direction::RightToLeft);
    buffer.set_script(script::ARABIC);
    let mut flags = BufferFlags::empty();
    if produce_tatweel {
        flags |= BufferFlags::PRODUCE_SAFE_TO_INSERT_TATWEEL;
    }
    buffer.set_flags(flags);

    let glyphs = shaper.shape(buffer, ShapeOptions::new());
    let infos = glyphs.glyph_infos();
    let pos = glyphs.glyph_positions();

    println!(
        "\n=== {label} : \"{text}\"  (chars: {}) produce_tatweel_flag={produce_tatweel}",
        text.chars().count()
    );
    println!(
        "  {:>3} {:>7} {:>7} {:>9} {:>8} {:>8}  flags",
        "vis", "gid", "cluster", "x_adv", "x_off", "y_off"
    );
    for (i, (gi, gp)) in infos.iter().zip(pos.iter()).enumerate() {
        println!(
            "  {:>3} {:>7} {:>7} {:>9} {:>8} {:>8}  {}",
            i,
            gi.glyph_id,
            gi.cluster,
            gp.x_advance,
            gp.x_offset,
            gp.y_offset,
            flag_str(gi.flags())
        );
    }
    let total: i32 = pos.iter().map(|p| p.x_advance).sum();
    println!("  total x_advance = {total} font units");
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_FONT.to_string());
    let font_bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read font {path:?}: {e}"));
    println!("Font: {path}");

    let font_ref = FontRef::from_index(&font_bytes, 0).unwrap();

    // --- Skrifa's role: cmap for U+0640 and its advance ---
    let charmap = font_ref.charmap();
    let tatweel_gid = charmap.map(TATWEEL);
    println!("\n--- Skrifa: U+0640 TATWEEL ---");
    println!("  cmap(U+0640) = {:?}", tatweel_gid);
    let upem = font_ref.metrics(Size::unscaled(), skrifa::instance::LocationRef::default())
        .units_per_em;
    println!("  units_per_em = {upem}");
    if let Some(gid) = tatweel_gid {
        let gm = font_ref.glyph_metrics(Size::unscaled(), skrifa::instance::LocationRef::default());
        println!("  advance_width(tatweel) = {:?} font units", gm.advance_width(gid));
    }

    // --- Probe 1: joining behaviour + tatweel flags ---
    // كتاب  = kaf + taa + alef + baa  (the word "kitab"/book)
    shape_and_print(&font_bytes, "kitab (no flag)", "كتاب", false);
    shape_and_print(&font_bytes, "kitab (tatweel flag)", "كتاب", true);

    // A longer connected word to expose more join opportunities.
    // يستطيعون  ("they can")
    shape_and_print(&font_bytes, "yastati3un (tatweel flag)", "يستطيعون", true);

    // --- Probe 2: naive post-shaping tatweel insertion vs re-shaping ---
    // Compare shaping "كتاب" against "كـتاب" (explicit U+0640 between kaf & taa).
    shape_and_print(&font_bytes, "kitab baseline", "كتاب", true);
    shape_and_print(&font_bytes, "k+TATWEEL+itab (reshaped)", "كـتاب", true);
    // And two tatweels
    shape_and_print(&font_bytes, "k+2xTATWEEL+itab (reshaped)", "كــتاب", true);
}
