// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Native build-time tool for Phase Four: reads `NotoColorEmoji-Regular.ttf` and emits a
//! tiny metrics-only "fake" font (`src/fake_noto.ttf`) with identical cmap coverage,
//! per-glyph advances, upem, and vertical metrics, but zero-contour glyphs (no glyph
//! image data at all: no `glyf` outlines, no `COLR/CPAL/SVG`). Also emits
//! `src/fake_noto_meta.rs` (gid <-> codepoint / advance cross-check tables) and runs the
//! mandatory native parity gate (Phase Four
//! plan §3.5 / §7.1): lay out example strings with the fake font and with real Noto,
//! through real Parley, and assert identical width / line metrics / baselines / per-emoji
//! advances.
//!
//! Not committed. Run manually:
//! ```sh
//! cargo run -p fake_font_gen
//! ```
//!
//! Note: earlier revisions of this generator also derived a Noto emoji "ink bounds" band
//! (via COLR layer bounds, with an empirical-ratio fallback) for a vertical-placement
//! scheme that fit the Apple capture's ink to that band. Per a team-lead ruling that
//! scheme was replaced by uniform-scale, baseline-on-baseline placement (see
//! `src/render.rs::blit_emoji`), which needs no Noto ink-bounds data at all, so that
//! machinery has been removed from this generator.

use std::path::PathBuf;

use read_fonts::TableProvider;
use skrifa::{FontRef, MetadataProvider};
use write_fonts::FontBuilder;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::glyf::{Glyf, Glyph as WGlyph, GlyfLocaBuilder};
use write_fonts::tables::head::{Head, MacStyle};
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::loca::Loca;
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::tables::post::Post;
use write_fonts::types::{FWord, GlyphId, NameId, Tag, UfWord};

const NOTO_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../../parley_dev/assets/fonts/noto_color_emoji/NotoColorEmoji-Regular.ttf"
);

const FAKE_FONT_OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/fake_noto.ttf");
const FAKE_META_OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/fake_noto_meta.rs");

fn main() {
    let bytes = std::fs::read(NOTO_PATH).unwrap_or_else(|e| panic!("failed to read {NOTO_PATH}: {e}"));
    let font = FontRef::new(&bytes).expect("failed to parse Noto font");

    let head = font.head().expect("no head table");
    let upem = head.units_per_em();
    let hhea = font.hhea().expect("no hhea table");
    let os2 = font.os2().expect("no OS/2 table");
    let hmtx = font.hmtx().expect("no hmtx table");
    let charmap = font.charmap();

    // Collect (cp, advance), sorted + deduped by codepoint, keeping ALL cmapped
    // codepoints (Q2 ruling).
    let mut rows: Vec<(u32, u16)> = Vec::new();
    for (cp, gid) in charmap.mappings() {
        let advance = hmtx.advance(gid).unwrap_or(0);
        rows.push((cp, advance));
    }
    rows.sort_by_key(|&(cp, _)| cp);
    rows.dedup_by_key(|&mut (cp, _)| cp);

    println!("Noto: upem={upem} cmap entries={} (kept all)", rows.len());

    // ---- assign compact gid space: gid 0 = .notdef, then one gid per kept codepoint ----
    // gid_to_cp[0] is a sentinel (0) for .notdef.
    let mut gid_to_cp: Vec<u32> = vec![0];
    let mut cp_to_advance: Vec<(u32, u16)> = Vec::with_capacity(rows.len());
    for &(cp, adv) in &rows {
        gid_to_cp.push(cp);
        cp_to_advance.push((cp, adv));
    }
    let num_glyphs = gid_to_cp.len();
    println!("fake font: num_glyphs={num_glyphs} (including .notdef)");

    // ---- build cmap: cp -> new gid ----
    let mappings: Vec<(char, GlyphId)> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, &(cp, _))| {
            char::from_u32(cp).map(|c| (c, GlyphId::new(u32::try_from(i + 1).unwrap())))
        })
        .collect();
    let cmap = Cmap::from_mappings(mappings).expect("no conflicting cmap mappings");

    // ---- build glyf/loca: all glyphs empty (zero contour) ----
    let mut glyf_builder = GlyfLocaBuilder::new();
    for _ in 0..num_glyphs {
        glyf_builder.add_glyph(&WGlyph::Empty).unwrap();
    }
    let (glyf, loca, loca_format): (Glyf, Loca, _) = glyf_builder.build();

    // ---- hmtx: advance per new gid (0 for .notdef) ----
    let h_metrics: Vec<LongMetric> = std::iter::once(LongMetric {
        advance: 0,
        side_bearing: 0,
    })
    .chain(cp_to_advance.iter().map(|&(_, adv)| LongMetric {
        advance: adv,
        side_bearing: 0,
    }))
    .collect();
    let hmtx_table = Hmtx {
        h_metrics,
        left_side_bearings: Vec::new(),
    };

    // ---- head ----
    let head_table = Head {
        units_per_em: upem,
        mac_style: MacStyle::empty(),
        lowest_rec_ppem: 6,
        index_to_loc_format: loca_format as i16,
        ..Default::default()
    };

    // ---- hhea (must match Noto exactly: drives Parley line metrics) ----
    let ascender = hhea.ascender().to_i16();
    let descender = hhea.descender().to_i16();
    let line_gap = hhea.line_gap().to_i16();
    let max_advance = cp_to_advance.iter().map(|&(_, a)| a).max().unwrap_or(0);
    let hhea_table = Hhea {
        ascender: FWord::new(ascender),
        descender: FWord::new(descender),
        line_gap: FWord::new(line_gap),
        advance_width_max: UfWord::new(max_advance),
        number_of_h_metrics: u16::try_from(num_glyphs).unwrap(),
        ..Default::default()
    };

    // ---- OS/2 (win + typo ascent/descent/line-gap must match Noto exactly) ----
    let os2_table = Os2 {
        us_weight_class: 400,
        us_width_class: 5,
        fs_type: 0,
        y_strikeout_size: os2.y_strikeout_size(),
        y_strikeout_position: os2.y_strikeout_position(),
        s_family_class: os2.s_family_class(),
        panose_10: os2.panose_10().try_into().unwrap_or([0; 10]),
        ach_vend_id: Tag::new(b"NONE"),
        fs_selection: SelectionFlags::empty(),
        us_first_char_index: rows.first().map(|&(c, _)| u16::try_from(c.min(0xFFFF)).unwrap_or(0)).unwrap_or(0),
        us_last_char_index: rows.last().map(|&(c, _)| u16::try_from(c.min(0xFFFF)).unwrap_or(0)).unwrap_or(0),
        s_typo_ascender: os2.s_typo_ascender(),
        s_typo_descender: os2.s_typo_descender(),
        s_typo_line_gap: os2.s_typo_line_gap(),
        us_win_ascent: os2.us_win_ascent(),
        us_win_descent: os2.us_win_descent(),
        ..Default::default()
    };

    // ---- maxp ----
    let maxp_table = Maxp {
        num_glyphs: u16::try_from(num_glyphs).unwrap(),
        ..Default::default()
    };

    // ---- name (minimal) ----
    let family = "FakeNotoEmoji";
    let mut name_records = vec![
        NameRecord::new(3, 1, 0x409, NameId::new(1), family.to_owned().into()),
        NameRecord::new(3, 1, 0x409, NameId::new(2), "Regular".to_owned().into()),
        NameRecord::new(3, 1, 0x409, NameId::new(4), family.to_owned().into()),
        NameRecord::new(3, 1, 0x409, NameId::new(6), family.to_owned().into()),
        NameRecord::new(1, 0, 0, NameId::new(1), family.to_owned().into()),
        NameRecord::new(1, 0, 0, NameId::new(2), "Regular".to_owned().into()),
        NameRecord::new(1, 0, 0, NameId::new(4), family.to_owned().into()),
        NameRecord::new(1, 0, 0, NameId::new(6), family.to_owned().into()),
    ];
    name_records.sort();
    let name_table = Name::new(name_records);

    // ---- post (format 3: no glyph names) ----
    let post_table = Post::default();

    let mut builder = FontBuilder::new();
    builder
        .add_table(&head_table)
        .unwrap()
        .add_table(&hhea_table)
        .unwrap()
        .add_table(&maxp_table)
        .unwrap()
        .add_table(&hmtx_table)
        .unwrap()
        .add_table(&cmap)
        .unwrap()
        .add_table(&name_table)
        .unwrap()
        .add_table(&os2_table)
        .unwrap()
        .add_table(&post_table)
        .unwrap()
        .add_table(&glyf)
        .unwrap()
        .add_table(&loca)
        .unwrap();
    let font_bytes = builder.build();

    let out_path = PathBuf::from(FAKE_FONT_OUT);
    std::fs::write(&out_path, &font_bytes).unwrap_or_else(|e| panic!("failed to write {out_path:?}: {e}"));
    println!(
        "wrote fake font: {} bytes ({:.1} KiB) to {out_path:?}",
        font_bytes.len(),
        font_bytes.len() as f64 / 1024.0
    );

    // Table size breakdown (parse back with read-fonts for a report).
    print_table_breakdown(&font_bytes);

    write_meta_rs(FAKE_META_OUT, upem, &gid_to_cp, &cp_to_advance);

    // ---- native parity gate (§3.5 / §7.1) ----
    run_parity_gate(&font_bytes, &bytes);
}

fn print_table_breakdown(font_bytes: &[u8]) {
    let f = FontRef::new(font_bytes).expect("failed to re-parse generated font");
    println!("table breakdown:");
    for record in f.table_directory().table_records() {
        println!("  {:>6} {:>8} bytes", record.tag(), record.length());
    }
}

fn write_meta_rs(out_path: &str, upem: u16, gid_to_cp: &[u32], cp_to_advance: &[(u32, u16)]) {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("// GENERATED by fake_font_gen — do not edit. (uncommitted)\n\n");
    let _ = writeln!(out, "pub(crate) const FAKE_UPEM: u16 = {upem};");
    let _ = writeln!(
        out,
        "/// `gid_to_cp[gid]` — gid 0 is `.notdef` (sentinel 0, not a real codepoint)."
    );
    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"reverse gid->cp table kept for cross-check / future non-source_char paths, not read on the current render path\")]"
    );
    let _ = writeln!(out, "pub(crate) const GID_TO_CP: &[u32] = &[");
    for cp in gid_to_cp {
        let _ = writeln!(out, "    0x{cp:X},");
    }
    out.push_str("];\n");
    let _ = writeln!(
        out,
        "/// (codepoint, `advance_in_font_units`), sorted ascending by codepoint."
    );
    let _ = writeln!(
        out,
        "#[allow(dead_code, reason = \"cp->advance cross-check table not read on the current render path (Cluster::advance() is used instead)\")]"
    );
    let _ = writeln!(out, "pub(crate) const CP_TO_ADVANCE: &[(u32, u16)] = &[");
    for &(cp, adv) in cp_to_advance {
        let _ = writeln!(out, "    (0x{cp:X}, {adv}),");
    }
    out.push_str("];\n");
    std::fs::write(out_path, out).unwrap_or_else(|e| panic!("failed to write {out_path}: {e}"));
    println!("wrote meta: {out_path}");
}

/// Native parity gate (Phase Four plan §3.5 / §7.1, MANDATORY). Lays out example strings
/// with (a) the fake font and (b) real Noto, both through real `parley`, and asserts
/// identical layout width, line metrics, `GlyphRun` baseline, and per-emoji cluster
/// advances. Panics (fails the build) if parity doesn't hold — per the team-lead ruling,
/// do NOT proceed to the wasm app if this fails.
fn run_parity_gate(fake_bytes: &[u8], real_bytes: &[u8]) {
    use std::sync::Arc;

    use parley::fontique::Blob;
    use parley::{FontContext, FontFamily, LayoutContext, PositionedLayoutItem, StyleProperty};

    const FONT_SIZE: f32 = 48.0;
    const TEST_STRINGS: &[&str] = &["\u{1F600}", "\u{1F389}", "Hi \u{1F600} world \u{1F389}!"];

    fn layout_metrics(
        font_bytes: &[u8],
        text: &str,
        font_size: f32,
    ) -> (f32, f32, f32, f32, Vec<f32>) {
        let mut font_cx = FontContext::new();
        let mut layout_cx: LayoutContext<()> = LayoutContext::new();
        let families = font_cx
            .collection
            .register_fonts(Blob::new(Arc::new(font_bytes.to_vec())), None);
        let family_name = families
            .first()
            .and_then(|(id, _)| font_cx.collection.family_name(*id))
            .map(str::to_owned)
            .expect("registered font has a family name");

        let mut builder = layout_cx.ranged_builder(&mut font_cx, text, 1.0, true);
        builder.push_default(FontFamily::named(family_name.as_str()));
        builder.push_default(StyleProperty::FontSize(font_size));
        let mut layout = builder.build(text);
        layout.break_all_lines(None);

        let width = layout.width();
        let height = layout.height();
        let mut baseline = 0.0_f32;
        let mut cluster_advances = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    baseline = run.baseline();
                    for cluster in run.run().clusters() {
                        cluster_advances.push(cluster.advance());
                    }
                }
            }
        }
        let ascent = layout.lines().next().unwrap().metrics().ascent;
        (width, height, baseline, ascent, cluster_advances)
    }

    let mut all_ok = true;
    for text in TEST_STRINGS {
        let (fw, fh, fbase, fasc, fadv) = layout_metrics(fake_bytes, text, FONT_SIZE);
        let (rw, rh, rbase, rasc, radv) = layout_metrics(real_bytes, text, FONT_SIZE);

        let width_ok = (fw - rw).abs() < 0.01;
        let height_ok = (fh - rh).abs() < 0.01;
        let base_ok = (fbase - rbase).abs() < 0.01;
        let asc_ok = (fasc - rasc).abs() < 0.01;
        let adv_ok = fadv.len() == radv.len()
            && fadv
                .iter()
                .zip(radv.iter())
                .all(|(a, b)| (a - b).abs() < 0.01);

        println!(
            "parity[{text:?}]: width fake={fw} real={rw} ok={width_ok} | height fake={fh} real={rh} ok={height_ok} | baseline fake={fbase} real={rbase} ok={base_ok} | ascent fake={fasc} real={rasc} ok={asc_ok} | advances_equal={adv_ok} (fake={fadv:?} real={radv:?})"
        );
        all_ok &= width_ok && height_ok && base_ok && asc_ok && adv_ok;
    }

    if all_ok {
        println!("NATIVE PARITY GATE: PASS");
    } else {
        panic!("NATIVE PARITY GATE: FAIL — see per-string diffs above; do not proceed to the wasm app");
    }
}
