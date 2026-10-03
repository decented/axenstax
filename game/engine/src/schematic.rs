//! Minecraft schematic import (#10) — read a Sponge `.schem` (gzipped NBT) and
//! convert it to an AxeNStax `PlanData` blueprint (then usable with the
//! build-guide / Vendor / etc.). The block palette is mapped best-effort: known
//! Minecraft blocks map to their AxeNStax equivalent, air is skipped, and
//! anything unmapped falls back to `STONE` so the **shape** is preserved.
//!
//! `.litematic` (a different, more complex packed-long format) is deferred —
//! `.schem` (Sponge v2/v3) is the common interchange format.
//!
//! Spec: `docs/foundations/2026-06-16-schematic-import.md`.

use crate::block;
use crate::block::BlockId;
use crate::nbt::{self, Nbt};
use crate::plan::{CapturedCell, PlanData};
use std::collections::HashMap;

/// Ceiling on a `.schem`'s decompressed NBT (matches `nbt::MAX_ALLOC_BYTES`).
pub const MAX_SCHEM_DECOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

/// Max per-axis size — `PlanData` dimensions are `u8`.
pub const MAX_AXIS: i32 = 255;

/// Decode a packed LEB128 varint stream (Sponge `BlockData`) into palette
/// indices. Each value is 7 bits/byte, little-endian, high bit = continue.
pub fn decode_varints(bytes: &[i8]) -> Vec<u32> {
    let mut out = Vec::new();
    let mut val: u32 = 0;
    let mut shift = 0u32;
    for &b in bytes {
        let byte = b as u8;
        // A varint longer than 5 bytes can't fit a u32; its extra bits are
        // dropped rather than shifted by ≥ 32 (a debug-build panic).
        val |= ((byte & 0x7f) as u32).checked_shl(shift).unwrap_or(0);
        if byte & 0x80 == 0 {
            out.push(val);
            val = 0;
            shift = 0;
        } else {
            shift = shift.saturating_add(7);
        }
    }
    out
}

/// Map a Minecraft block-state string (e.g. `minecraft:oak_stairs[facing=north]`)
/// to an AxeNStax block. `None` = air (skip the cell). Unmapped non-air blocks
/// fall back to `STONE` so the structure's shape survives the import.
pub fn map_mc_block(state: &str) -> Option<BlockId> {
    // Strip the namespace + any "[properties]".
    let name = state
        .strip_prefix("minecraft:")
        .unwrap_or(state)
        .split('[')
        .next()
        .unwrap_or("");
    match name {
        "air" | "cave_air" | "void_air" => None,
        "stone" | "granite" | "diorite" | "andesite" | "polished_granite"
        | "polished_diorite" | "polished_andesite" | "smooth_stone" => Some(block::STONE),
        "cobblestone" | "mossy_cobblestone" => Some(block::COBBLESTONE),
        "dirt" | "coarse_dirt" | "rooted_dirt" | "podzol" | "farmland" | "dirt_path" => {
            Some(block::DIRT)
        }
        "grass_block" => Some(block::GRASS),
        "sand" | "red_sand" | "soul_sand" | "soul_soil" => Some(block::SAND),
        "gravel" => Some(block::GRAVEL),
        "water" => Some(block::WATER),
        "bedrock" => Some(block::BEDROCK),
        "glass" | "white_stained_glass" | "tinted_glass" => Some(block::GLASS),
        "bricks" => Some(block::COBBLESTONE),
        n if n.ends_with("_planks") => Some(block::OAK_PLANKS),
        n if n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") => {
            Some(block::OAK_LOG)
        }
        n if n.ends_with("_leaves") => Some(block::OAK_LEAVES),
        // Unmapped non-air: keep the shape with stone.
        _ => Some(block::STONE),
    }
}

/// Convert a parsed `.schem` NBT root into a `PlanData`. Handles Sponge v2
/// (palette + `BlockData` at the schematic level) and v3 (a `Blocks` compound
/// holding `Palette` + `Data`).
pub fn schem_to_plan(root: &Nbt, name: String) -> Result<PlanData, String> {
    // v3 nests everything under "Schematic"; v2 has it at the root.
    let c = root.get("Schematic").unwrap_or(root);

    let w = c.get("Width").and_then(Nbt::as_i16).ok_or("missing Width")? as i32;
    let h = c.get("Height").and_then(Nbt::as_i16).ok_or("missing Height")? as i32;
    let l = c.get("Length").and_then(Nbt::as_i16).ok_or("missing Length")? as i32;
    if w <= 0 || h <= 0 || l <= 0 {
        return Err("non-positive dimensions".to_string());
    }
    if w > MAX_AXIS || h > MAX_AXIS || l > MAX_AXIS {
        return Err(format!("schematic too large (max {MAX_AXIS} per axis)"));
    }

    // Palette (index → state) + block data: try v3 `Blocks{Palette,Data}` first,
    // then v2 `Palette` + `BlockData`.
    let (palette_tag, data_tag) = if let Some(blocks) = c.get("Blocks") {
        (
            blocks.get("Palette").ok_or("missing Blocks.Palette")?,
            blocks.get("Data").ok_or("missing Blocks.Data")?,
        )
    } else {
        (
            c.get("Palette").ok_or("missing Palette")?,
            c.get("BlockData").ok_or("missing BlockData")?,
        )
    };

    let palette_map = palette_tag.as_compound().ok_or("Palette is not a compound")?;
    // Invert: palette index → blockstate string.
    let mut by_index: HashMap<u32, &str> = HashMap::new();
    for (state, idx) in palette_map {
        if let Some(i) = idx.as_i32() {
            by_index.insert(i as u32, state.as_str());
        }
    }

    let data = data_tag.as_byte_array().ok_or("block data is not a byte array")?;
    let indices = decode_varints(data);

    let mut cells = Vec::new();
    for (i, &pidx) in indices.iter().enumerate() {
        let Some(&state) = by_index.get(&pidx) else {
            continue; // index not in palette — skip
        };
        let Some(blk) = map_mc_block(state) else {
            continue; // air
        };
        let i = i as i32;
        // Sponge index order is YZX: i = x + z*W + y*W*L.
        let x = i % w;
        let z = (i / w) % l;
        let y = i / (w * l);
        cells.push(CapturedCell {
            rx: x as u8,
            ry: y as u8,
            rz: z as u8,
            block_id: blk,
        });
    }

    Ok(PlanData::from_imported(
        name, w as u8, l as u8, h as u8, cells,
    ))
}

/// Full import path: gunzip the `.schem` bytes, NBT-parse, convert to a plan.
pub fn parse_schem(gzipped: &[u8], name: String) -> Result<PlanData, String> {
    parse_schem_capped(gzipped, name, MAX_SCHEM_DECOMPRESSED_BYTES)
}

fn parse_schem_capped(gzipped: &[u8], name: String, cap: u64) -> Result<PlanData, String> {
    // Bounded gunzip: a small gzip bomb must fail, not OOM (audit 2026-09-27).
    let decoder = flate2::read::GzDecoder::new(gzipped);
    let buf = crate::save::read_bounded(decoder, cap)
        .map_err(|e| format!("gunzip failed: {e}"))?;
    let (_root_name, root) = nbt::parse(&buf).map_err(|e| format!("NBT parse failed: {e:?}"))?;
    schem_to_plan(&root, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_decode_multibyte_values() {
        // 0x05 → 5; [0xAC,0x02] → 0x2C | (0x02<<7) = 300; 0x00 → 0.
        let bytes: Vec<i8> = vec![5, 0xACu8 as i8, 0x02, 0];
        assert_eq!(decode_varints(&bytes), vec![5, 300, 0]);
    }

    #[test]
    fn block_mapping_handles_air_known_and_unknown() {
        assert_eq!(map_mc_block("minecraft:air"), None);
        assert_eq!(map_mc_block("minecraft:stone"), Some(block::STONE));
        assert_eq!(map_mc_block("minecraft:oak_planks"), Some(block::OAK_PLANKS));
        assert_eq!(
            map_mc_block("minecraft:spruce_planks"),
            Some(block::OAK_PLANKS)
        );
        assert_eq!(
            map_mc_block("minecraft:oak_stairs[facing=north]"),
            Some(block::STONE),
            "unmapped non-air keeps the shape as stone"
        );
    }

    #[test]
    fn schem_to_plan_decodes_palette_and_positions() {
        // A 2×1×1 schematic: index 0 = stone (palette 0), index 1 = air (palette 1).
        let mut palette = HashMap::new();
        palette.insert("minecraft:stone".to_string(), Nbt::Int(0));
        palette.insert("minecraft:air".to_string(), Nbt::Int(1));
        let mut root_map = HashMap::new();
        root_map.insert("Width".to_string(), Nbt::Short(2));
        root_map.insert("Height".to_string(), Nbt::Short(1));
        root_map.insert("Length".to_string(), Nbt::Short(1));
        root_map.insert("Palette".to_string(), Nbt::Compound(palette));
        // BlockData: varints [0, 1] (cell 0 = stone, cell 1 = air).
        root_map.insert("BlockData".to_string(), Nbt::ByteArray(vec![0, 1]));
        let root = Nbt::Compound(root_map);

        let plan = schem_to_plan(&root, "Imported".to_string()).unwrap();
        assert_eq!(plan.width, 2);
        assert_eq!(plan.height, 1);
        assert_eq!(plan.depth, 1);
        // Only the stone cell survives (air skipped); it's at (0,0,0).
        assert_eq!(plan.cells.len(), 1);
        assert_eq!(plan.cells[0].rx, 0);
        assert_eq!(plan.cells[0].block_id, block::STONE);
    }

    #[test]
    fn schem_rejects_oversize() {
        let mut root_map = HashMap::new();
        root_map.insert("Width".to_string(), Nbt::Short(300)); // > 255
        root_map.insert("Height".to_string(), Nbt::Short(1));
        root_map.insert("Length".to_string(), Nbt::Short(1));
        root_map.insert("Palette".to_string(), Nbt::Compound(HashMap::new()));
        root_map.insert("BlockData".to_string(), Nbt::ByteArray(vec![]));
        let root = Nbt::Compound(root_map);
        assert!(schem_to_plan(&root, "x".to_string()).is_err());
    }

    #[test]
    fn an_overlong_varint_does_not_panic() {
        // Nine continuation bytes then a terminator: shift passes 32.
        let bytes: Vec<i8> = [0xffu8; 9].iter().map(|&b| b as i8).chain([0x01]).collect();
        let out = decode_varints(&bytes);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_gzip_bomb_is_refused_not_inflated() {
        use std::io::Write;
        // Same code path as `parse_schem`, with a 1 MiB cap so the test is fast.
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&vec![0u8; 2 * 1024 * 1024]).unwrap();
        let gz = enc.finish().unwrap();
        let err = parse_schem_capped(&gz, "bomb".into(), 1024 * 1024).unwrap_err();
        assert!(err.contains("gunzip"), "{err}");
    }
}
