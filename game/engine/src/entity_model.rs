//! Entity model system — multi-cuboid mob models with textures and walk animation.
//!
//! Each mob is defined as a list of cuboid parts (head, body, legs, etc).
//! Parts have per-face texture layers into the block texture array and
//! pivot points for animation. Walk animation rotates legs around their pivots.

use glam::Vec3;
use crate::mesh::Vertex;
use crate::mob::MobType;
use crate::entity::{Hitbox, ItemEntity, MobKind, Position, Velocity};
use crate::item::Item;
use crate::mob_ai::MobAi;
use crate::combat::Health;

/// Overlay inflation for the RIGHT ARM specifically — the one part the
/// first-person viewmodel draws (`crate::viewmodel`). Per-part overlay inflate
/// now lives in [`crate::skin_pose`] (Minecraft-exact: head 0.03125, limbs
/// 0.015625 per side); new code should call `skin_pose::overlay_inflate(part)`
/// directly rather than reaching for a constant.
pub(crate) const RIGHT_ARM_OVERLAY_INFLATE: f32 = 0.015625;

/// How to build an avatar's skin mesh. [`Default`] is the WORN avatar: parts at
/// rest, Minecraft-exact overlay inflate, clothes drawn.
#[derive(Clone, Copy, Debug)]
pub struct SkinRenderOpts {
    /// Eased limbs-apart factor, 0 = together, 1 = fully apart. MUST match the
    /// value handed to `skin_hit::ray_hit_avatar` or the crosshair and the limb
    /// disagree.
    pub separation: f32,
    /// Use the exaggerated Workshop overlay inflate instead of the true one.
    /// Blown-up mannequin only — never a worn avatar, which must stay
    /// Minecraft-exact so a skin looks the same here and in Minecraft.
    pub edit_inflate: bool,
    /// Draw the overlay (clothes) shell at all.
    pub draw_overlay: bool,
    /// Which arm shape to build — Minecraft's Classic (4 px) or Slim (3 px).
    /// MUST match what `skin_hit::ray_hit_avatar` and `skin_grid` are given, or
    /// the crosshair lands beside the arm on a slim avatar.
    pub arm_model: crate::skin_uv::ArmModel,
}

impl Default for SkinRenderOpts {
    fn default() -> Self {
        Self {
            separation: 0.0,
            edit_inflate: false,
            draw_overlay: true,
            arm_model: crate::skin_uv::ArmModel::Classic,
        }
    }
}

/// Texture layer offsets for mob textures (appended after block textures).
/// Must match the order in texture_gen::generate_textures().
pub const TEX_ENTITY_BASE: u32 = 16; // First mob texture layer

// Cow textures
pub const TEX_COW_HEAD_FRONT: u32 = TEX_ENTITY_BASE;
pub const TEX_COW_HEAD_SIDE: u32 = TEX_ENTITY_BASE + 1;
pub const TEX_COW_HEAD_TOP: u32 = TEX_ENTITY_BASE + 2;
pub const TEX_COW_BODY_SIDE: u32 = TEX_ENTITY_BASE + 3;
pub const TEX_COW_BODY_TOP: u32 = TEX_ENTITY_BASE + 4;
pub const TEX_COW_BODY_END: u32 = TEX_ENTITY_BASE + 5;
pub const TEX_COW_LEG: u32 = TEX_ENTITY_BASE + 6;

// Retired layers 23-29 (TEX_ENTITY_BASE + 7..=13). The generators are still
// pushed (as neutral grey fillers) to keep all later layer indices stable;
// the named consts were removed in the fantasy-roster excision.

// Chicken textures
pub const TEX_CHICKEN_HEAD: u32 = TEX_ENTITY_BASE + 14;
pub const TEX_CHICKEN_BODY: u32 = TEX_ENTITY_BASE + 15;
pub const TEX_CHICKEN_LEG: u32 = TEX_ENTITY_BASE + 16;
pub const TEX_CHICKEN_WING: u32 = TEX_ENTITY_BASE + 17;

// Pig textures
pub const TEX_PIG_HEAD_FRONT: u32 = TEX_ENTITY_BASE + 18;
pub const TEX_PIG_HEAD_SIDE: u32 = TEX_ENTITY_BASE + 19;
pub const TEX_PIG_HEAD_TOP: u32 = TEX_ENTITY_BASE + 20;
pub const TEX_PIG_BODY_SIDE: u32 = TEX_ENTITY_BASE + 21;
pub const TEX_PIG_BODY_TOP: u32 = TEX_ENTITY_BASE + 22;
pub const TEX_PIG_BODY_END: u32 = TEX_ENTITY_BASE + 23;
pub const TEX_PIG_LEG: u32 = TEX_ENTITY_BASE + 24;

// Sheep textures
pub const TEX_SHEEP_HEAD_FRONT: u32 = TEX_ENTITY_BASE + 25;
pub const TEX_SHEEP_HEAD_SIDE: u32 = TEX_ENTITY_BASE + 26;
pub const TEX_SHEEP_HEAD_TOP: u32 = TEX_ENTITY_BASE + 27;
pub const TEX_SHEEP_BODY_SIDE: u32 = TEX_ENTITY_BASE + 28;
pub const TEX_SHEEP_BODY_TOP: u32 = TEX_ENTITY_BASE + 29;
pub const TEX_SHEEP_BODY_END: u32 = TEX_ENTITY_BASE + 30;
pub const TEX_SHEEP_LEG: u32 = TEX_ENTITY_BASE + 31;

// Retired layers 48-66 (TEX_ENTITY_BASE + 32..=50). Generators retained as
// neutral grey fillers to keep later layer indices stable; named consts
// removed in the fantasy-roster excision.

// Item-drop textures (Wave 2b) — used by build_item_entity_vertices when an
// ItemEntity carries a non-block stack (Material/Tool). Block items still use
// their underlying block textures.
pub const TEX_ITEM_STICK: u32 = TEX_ENTITY_BASE + 51;
pub const TEX_ITEM_LEATHER: u32 = TEX_ENTITY_BASE + 52;
pub const TEX_ITEM_FEATHER: u32 = TEX_ENTITY_BASE + 53;
pub const TEX_ITEM_WOOL: u32 = TEX_ENTITY_BASE + 54;
pub const TEX_ITEM_BONE: u32 = TEX_ENTITY_BASE + 55;
// Layer 72 (TEX_ENTITY_BASE + 56) retired — neutral grey filler generator
// retained to keep later indices stable; named const removed.
pub const TEX_ITEM_RAW_BEEF: u32 = TEX_ENTITY_BASE + 57;
pub const TEX_ITEM_RAW_PORKCHOP: u32 = TEX_ENTITY_BASE + 58;
pub const TEX_ITEM_RAW_CHICKEN: u32 = TEX_ENTITY_BASE + 59;
pub const TEX_ITEM_RAW_MUTTON: u32 = TEX_ENTITY_BASE + 60;
pub const TEX_ITEM_STRING: u32 = TEX_ENTITY_BASE + 61;
// Layer 78 (TEX_ENTITY_BASE + 62) retired — neutral grey filler generator
// retained to keep later indices stable; named const removed.
// Generic grey-powder texture (layer 79). Reused by Sulphur, InkSac, BeeStinger.
pub const TEX_ITEM_GREY_POWDER: u32 = TEX_ENTITY_BASE + 63;

// Per-tool-tier textures (Wave 2c). One per ToolMaterial; ToolType is not
// differentiated on the ground (pickup → hotbar shows the proper name).
pub const TEX_ITEM_TOOL_WOOD: u32 = TEX_ENTITY_BASE + 64;
pub const TEX_ITEM_TOOL_STONE: u32 = TEX_ENTITY_BASE + 65;
pub const TEX_ITEM_TOOL_IRON: u32 = TEX_ENTITY_BASE + 66;
pub const TEX_ITEM_TOOL_DIAMOND: u32 = TEX_ENTITY_BASE + 67;

// Retired layers 89-92 (formerly a fantasy mob + its drop). They sit AFTER
// the bed (84-85) and ore (86-88) block textures. Generators retained as
// neutral grey fillers to keep later indices stable; named consts removed
// in the fantasy-roster excision.

// Smelting outputs (Wave 6). Slot in after glass (93), storage blocks
// (94-96), and torch (97).
pub const TEX_ITEM_IRON_INGOT: u32 = 98;
pub const TEX_ITEM_COOKED_BEEF: u32 = 99;
pub const TEX_ITEM_COOKED_PORKCHOP: u32 = 100;
pub const TEX_ITEM_COOKED_CHICKEN: u32 = 101;
pub const TEX_ITEM_COOKED_MUTTON: u32 = 102;

// Wave 23: Arrow item (drop) + projectile-arrow (in-flight). Reuses the same
// texture for both — the in-flight render scales it to a thin cuboid.
pub const TEX_ITEM_ARROW: u32 = 105;

// Spec 19 — Villager / Iron Golem / Wandering Villager (layers 146-157).
// Sit after the campfire-ext block (136-145) per texture_gen's append order.
pub const TEX_VILLAGER_HEAD_FRONT: u32 = 146;
pub const TEX_VILLAGER_HEAD_SIDE: u32 = 147;
pub const TEX_VILLAGER_BODY: u32 = 148;
pub const TEX_VILLAGER_LEG: u32 = 149;
// Retired layers 150-153 (formerly a fantasy construct mob). Generators
// retained as neutral grey fillers to keep later indices stable; named
// consts removed in the fantasy-roster excision.
pub const TEX_WANDERING_HEAD_FRONT: u32 = 154;
pub const TEX_WANDERING_HEAD_SIDE: u32 = 155;
pub const TEX_WANDERING_BODY: u32 = 156;
pub const TEX_WANDERING_LEG: u32 = 157;

// Satoshi the founder-sage (2026-06-25 — layers 441-443, appended LAST in
// texture_gen so existing indices are undisturbed). A brown hooded head (the
// cloak read is textural, like the Peddler) + a pale glow trim for his amulet.
pub const TEX_SATOSHI_HEAD_FRONT: u32 = 441;
pub const TEX_SATOSHI_HEAD_SIDE: u32 = 442;
pub const TEX_SATOSHI_TRIM: u32 = 443;

// `ENTITY_TEXTURE_COUNT` was removed here — zero references anywhere;
// `texture_gen::texture_count()` is the actual total-layer-count source of
// truth, computed independently of this stale constant.


// Player skin layers 201-206 — RETIRED to grey fillers. The player now renders
// via the 64x64 skin path (`skin_texture` atlas + `skin_uv` UV tables, used by
// `build_skin_part_vertices` for the 3rd-person avatar and `build_skin_part_local`
// for the 1st-person viewmodel arm), which ignores `part.tex_faces`. These consts
// are KEPT only because `PLAYER_MODEL` still names them in its `tex_faces` arrays;
// that `tex_faces` is now VESTIGIAL for the player (unread — no renderer samples
// these layers). The field still matters for mobs (cow/pig/sheep/etc.), which
// render via the block-array path. See texture_gen.rs (layers 201-206 fillers).
pub const TEX_PLAYER_HEAD_FRONT: u32 = 201;
pub const TEX_PLAYER_HEAD_SIDE: u32 = 202;
pub const TEX_PLAYER_HEAD_TOP: u32 = 203;
pub const TEX_PLAYER_BODY: u32 = 204;
pub const TEX_PLAYER_ARM: u32 = 205;
pub const TEX_PLAYER_LEG: u32 = 206;

// Bee + Squid bespoke coats (2026-05-27 — layers 217-219). Sit after the
// crack-overlay block (207-216) in texture_gen's append order. Replace the
// chicken/sheep stand-ins the first mob-model pass borrowed.
pub const TEX_BEE_BODY: u32 = 217;
pub const TEX_BEE_HEAD: u32 = 218;
pub const TEX_SQUID_BODY: u32 = 219;

// Spec 35/36 item textures (2026-05-27 — layers 225-230). Plant block
// textures (220-224) live in block.rs. Dye + fibre + rope item icons:
pub const TEX_ITEM_BLUE_DYE: u32 = 225;
pub const TEX_ITEM_RED_DYE: u32 = 226;
pub const TEX_ITEM_YELLOW_DYE: u32 = 227;
pub const TEX_ITEM_COTTON: u32 = 228;
pub const TEX_ITEM_HEMP_FIBRE: u32 = 229;
pub const TEX_ITEM_ROPE: u32 = 230;

// Spec 37 Magnesium item icons (2026-05-27 — layers 232-236; the ore block
// texture 231 lives in block.rs).
pub const TEX_ITEM_MAGNESIUM: u32 = 232;
pub const TEX_ITEM_FERTILISER: u32 = 233;
pub const TEX_ITEM_SPARKLER: u32 = 234;
pub const TEX_ITEM_FLARE: u32 = 235;
pub const TEX_ITEM_FIRESTARTER: u32 = 236;

// Dye Phase 2 item icons (Spec 35, 2026-05-27 — layers 237-246).
pub const TEX_ITEM_BLACK_DYE: u32 = 237;
pub const TEX_ITEM_WHITE_DYE: u32 = 238;
pub const TEX_ITEM_ORANGE_DYE: u32 = 239;
pub const TEX_ITEM_GREEN_DYE: u32 = 240;
pub const TEX_ITEM_PURPLE_DYE: u32 = 241;
pub const TEX_ITEM_PINK_DYE: u32 = 242;
pub const TEX_ITEM_LIME_DYE: u32 = 243;
pub const TEX_ITEM_LIGHT_BLUE_DYE: u32 = 244;
pub const TEX_ITEM_GREY_DYE: u32 = 245;
pub const TEX_ITEM_LIGHT_GREY_DYE: u32 = 246;
// Spec 35 Phase 2 completion (2026-05-28) — 3-input mix dyes.
pub const TEX_ITEM_BROWN_DYE: u32 = 270;
pub const TEX_ITEM_CYAN_DYE: u32 = 271;
pub const TEX_ITEM_MAGENTA_DYE: u32 = 272;
// Spec 36 Phase 2 (2026-05-28) — Rope-consumer + textiles. Layers
// 276-278 sit just after the wallpaper textures at 273-275.
pub const TEX_ITEM_LEAD: u32 = 276;
pub const TEX_ITEM_CLOTH: u32 = 277;
pub const TEX_ITEM_CANVAS: u32 = 278;
// Spec 35 farmable-flower follow-on (2026-05-28). Seed icons at
// 288-290; the 9 crop-stage block textures live in `block.rs`
// (279-287).
pub const TEX_ITEM_CORNFLOWER_SEEDS: u32 = 288;
pub const TEX_ITEM_FIELD_POPPY_SEEDS: u32 = 289;
pub const TEX_ITEM_BUTTERCUP_SEEDS: u32 = 290;

// Fibre Phase 2 seed icons (Spec 36, 2026-05-27 — layers 255-256; crop-stage
// block textures 247-254 live in block.rs).
pub const TEX_ITEM_COTTON_SEEDS: u32 = 255;
pub const TEX_ITEM_HEMP_SEEDS: u32 = 256;

/// A cuboid part of a mob model.
#[derive(Clone, Debug)]
pub struct ModelPart {
    /// Offset from entity foot position (centre-bottom of part).
    pub origin: Vec3,
    /// Size in blocks (width_x, height_y, depth_z).
    pub size: Vec3,
    /// Pivot point for rotation (relative to entity foot position).
    pub pivot: Vec3,
    /// Whether this part animates with walk cycle.
    pub animated: bool,
    /// Animation phase offset (0.0 or 0.5 for alternating legs).
    pub phase: f32,
    /// Texture layers: [+x, -x, +y, -y, +z, -z] (right, left, top, bottom, front, back)
    pub tex_faces: [u32; 6],
    /// Whether this part pitches up/down with the entity's look direction
    /// (player head tracking look pitch). Default false for all mobs.
    pub pitch_tracks_look: bool,
}

/// Cached model parts per mob type — built once, reused every frame.
static MODEL_CACHE: std::sync::LazyLock<std::collections::HashMap<MobType, Vec<ModelPart>>> =
    std::sync::LazyLock::new(|| {
        let mut m = std::collections::HashMap::new();
        m.insert(MobType::Cow, cow_model());
        m.insert(MobType::Chicken, chicken_model());
        m.insert(MobType::Pig, pig_model());
        m.insert(MobType::Sheep, sheep_model());
        m.insert(MobType::Villager, villager_model());
        m.insert(MobType::Peddler, peddler_model());
        m.insert(MobType::Bear, bear_model());
        m.insert(MobType::Hyena, hyena_model());
        // 2026-05-27 — these seven were spawning correctly but had no
        // model, so the renderer drew nothing (invisible animals). Each
        // borrows the closest existing coat texture (cow = brown, sheep
        // = sandy/cream, chicken = pale) the way Bear/Hyena do; proper
        // bespoke textures (esp. bee-yellow + squid-blue) are a
        // playtest-driven polish pass.
        m.insert(MobType::Horse, horse_model());
        m.insert(MobType::Wolf, wolf_model());
        m.insert(MobType::Rabbit, rabbit_model());
        m.insert(MobType::Goat, goat_model());
        m.insert(MobType::Nostrich, nostrich_model());
        m.insert(MobType::Bee, bee_model());
        m.insert(MobType::Squid, squid_model());
        // Aquatic wave — Fish + Shark get small proc meshes; Glow Squid reuses
        // the squid mesh with its cyan tinted coat (mob-species-tint
        // foundation, 2026-07-11 — identity really does read from
        // MobDef.color now, baked into per-species texture layers). Bespoke
        // textures (grey shark) are a playtest polish pass.
        m.insert(MobType::Fish, fish_model());
        m.insert(MobType::Shark, shark_model());
        m.insert(MobType::GlowSquid, retint(MobType::GlowSquid, squid_model()));
        // Wild fauna wave — reuse the closest existing quadruped/canine meshes
        // with per-species tinted coats (orange fox, white polar bear, brown
        // reindeer — MobDef.color baked, 2026-07-11). Bespoke meshes (esp.
        // antlers) are a polish pass.
        m.insert(MobType::Fox, retint(MobType::Fox, wolf_model()));
        m.insert(MobType::PolarBear, retint(MobType::PolarBear, bear_model()));
        m.insert(MobType::Reindeer, retint(MobType::Reindeer, horse_model()));
        // Companions wave — Cat reuses the (small) wolf canine mesh; Parrot the
        // chicken bird mesh; both tinted (grey cat, red parrot — 2026-07-11).
        // Bespoke meshes are a polish pass.
        m.insert(MobType::Cat, retint(MobType::Cat, wolf_model()));
        m.insert(MobType::Parrot, retint(MobType::Parrot, chicken_model()));
        // Logistics wave — donkey + mule reuse the horse mesh, tinted brown
        // (2026-07-11).
        m.insert(MobType::Donkey, retint(MobType::Donkey, horse_model()));
        m.insert(MobType::Mule, retint(MobType::Mule, horse_model()));
        // HP-3 — Brigand / Marauder / Berserker share the villager
        // bipedal mesh; tier identity reads from `MobDef.color` (per-
        // TOML tint: brown / grey / rust-red). When the dedicated
        // armoured-human mesh lands post-playtest, replace these three.
        m.insert(MobType::Brigand, villager_model());
        m.insert(MobType::Marauder, villager_model());
        m.insert(MobType::Berserker, villager_model());
        // HP-4 — Knight reuses the villager mesh; identity reads from
        // `MobDef.color` (steel grey). Dedicated armoured mesh deferred
        // to post-playtest polish.
        m.insert(MobType::Knight, villager_model());
        // Pets wave Task 13 — Crab reuses the (small) rabbit mesh, tinted
        // rust-red (2026-07-11). A bespoke low, wide crab mesh is a
        // playtest-driven polish pass.
        m.insert(MobType::Crab, retint(MobType::Crab, rabbit_model()));
        m
    });

/// Mob-species-tint foundation (2026-07-11) — swap a mesh-reuse species'
/// donor coat layers for its own tinted run (`texture_gen::SPECIES_TINTS`,
/// tint = `MobDef.color`). Faces without a tint mapping keep the donor layer,
/// so a species absent from the table is a straight mesh reuse as before.
fn retint(kind: MobType, mut model: Vec<ModelPart>) -> Vec<ModelPart> {
    for part in &mut model {
        for face in &mut part.tex_faces {
            if let Some(tinted) = crate::texture_gen::tinted_layer(kind, *face) {
                *face = tinted;
            }
        }
    }
    model
}

/// Get the model parts for a mob type (cached — no allocation after first call).
pub fn mob_model(kind: MobType) -> &'static [ModelPart] {
    MODEL_CACHE.get(&kind).map(|v| v.as_slice()).unwrap_or(&[])
}

/// Cached Satoshi model (one per world, but built once). Satoshi is a
/// `MobType::Villager` + a `SatoshiMarker`; `build_entity_model_vertices`
/// swaps this in for the marker rather than keying a new `MobType`.
static SATOSHI_MODEL: std::sync::LazyLock<Vec<ModelPart>> =
    std::sync::LazyLock::new(satoshi_model);

/// Vertex-`light` sentinel that marks a self-glowing (emissive) part. The
/// fragment shader (`shader.wgsl` `fs_main`) renders any vertex with
/// `light > 1.5` mostly unlit, so it glows even in a dim hut. All other
/// geometry uses `light <= 1.0`, so the sentinel is inert for everything else.
pub const SATOSHI_GLOW_LIGHT: f32 = 2.0;

fn cow_model() -> Vec<ModelPart> {
    vec![
        // Body
        ModelPart {
            origin: Vec3::new(0.0, 0.65, 0.0),
            size: Vec3::new(0.5625, 0.45, 0.8),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_COW_BODY_SIDE, TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_TOP, TEX_COW_BODY_END, TEX_COW_BODY_END],
            pitch_tracks_look: false,
        },
        // Head
        ModelPart {
            origin: Vec3::new(0.0, 0.9, -0.5),
            size: Vec3::new(0.5, 0.5, 0.375),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_COW_HEAD_SIDE, TEX_COW_HEAD_SIDE, TEX_COW_HEAD_TOP, TEX_COW_HEAD_TOP, TEX_COW_HEAD_FRONT, TEX_COW_HEAD_FRONT],
            pitch_tracks_look: false,
        },
        // Front-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.3, -0.25),
            size: Vec3::new(0.15, 0.6, 0.15),
            pivot: Vec3::new(-0.15, 0.6, -0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_COW_LEG; 6],
            pitch_tracks_look: false,
        },
        // Front-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.3, -0.25),
            size: Vec3::new(0.15, 0.6, 0.15),
            pivot: Vec3::new(0.15, 0.6, -0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_COW_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.3, 0.25),
            size: Vec3::new(0.15, 0.6, 0.15),
            pivot: Vec3::new(-0.15, 0.6, 0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_COW_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.3, 0.25),
            size: Vec3::new(0.15, 0.6, 0.15),
            pivot: Vec3::new(0.15, 0.6, 0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_COW_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

fn pig_model() -> Vec<ModelPart> {
    // Squat, lower-slung version of the cow shape — matches data/mobs/pig.toml
    // height 0.9 / width 0.9.
    vec![
        // Body
        ModelPart {
            origin: Vec3::new(0.0, 0.45, 0.0),
            size: Vec3::new(0.5625, 0.4, 0.8),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_PIG_BODY_SIDE, TEX_PIG_BODY_SIDE, TEX_PIG_BODY_TOP, TEX_PIG_BODY_TOP, TEX_PIG_BODY_END, TEX_PIG_BODY_END],
            pitch_tracks_look: false,
        },
        // Head
        ModelPart {
            origin: Vec3::new(0.0, 0.55, -0.5),
            size: Vec3::new(0.5, 0.45, 0.4),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_PIG_HEAD_SIDE, TEX_PIG_HEAD_SIDE, TEX_PIG_HEAD_TOP, TEX_PIG_HEAD_TOP, TEX_PIG_HEAD_FRONT, TEX_PIG_HEAD_FRONT],
            pitch_tracks_look: false,
        },
        // Front-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.15, -0.25),
            size: Vec3::new(0.15, 0.3, 0.15),
            pivot: Vec3::new(-0.15, 0.3, -0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Front-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.15, -0.25),
            size: Vec3::new(0.15, 0.3, 0.15),
            pivot: Vec3::new(0.15, 0.3, -0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.15, 0.25),
            size: Vec3::new(0.15, 0.3, 0.15),
            pivot: Vec3::new(-0.15, 0.3, 0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.15, 0.25),
            size: Vec3::new(0.15, 0.3, 0.15),
            pivot: Vec3::new(0.15, 0.3, 0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

fn sheep_model() -> Vec<ModelPart> {
    // Cow-like proportions but with a fluffier body and slightly forward head —
    // matches data/mobs/sheep.toml height 1.3 / width 0.9.
    vec![
        // Body (fluffier — slightly larger than cow)
        ModelPart {
            origin: Vec3::new(0.0, 0.7, 0.0),
            size: Vec3::new(0.625, 0.55, 0.85),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END, TEX_SHEEP_BODY_END],
            pitch_tracks_look: false,
        },
        // Head
        ModelPart {
            origin: Vec3::new(0.0, 0.85, -0.5),
            size: Vec3::new(0.4, 0.4, 0.4),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_FRONT, TEX_SHEEP_HEAD_FRONT],
            pitch_tracks_look: false,
        },
        // Front-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.25, -0.25),
            size: Vec3::new(0.15, 0.5, 0.15),
            pivot: Vec3::new(-0.15, 0.5, -0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Front-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.25, -0.25),
            size: Vec3::new(0.15, 0.5, 0.15),
            pivot: Vec3::new(0.15, 0.5, -0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-left leg
        ModelPart {
            origin: Vec3::new(-0.15, 0.25, 0.25),
            size: Vec3::new(0.15, 0.5, 0.15),
            pivot: Vec3::new(-0.15, 0.5, 0.25),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-right leg
        ModelPart {
            origin: Vec3::new(0.15, 0.25, 0.25),
            size: Vec3::new(0.15, 0.5, 0.15),
            pivot: Vec3::new(0.15, 0.5, 0.25),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

fn chicken_model() -> Vec<ModelPart> {
    vec![
        // Body
        ModelPart {
            origin: Vec3::new(0.0, 0.3, 0.0),
            size: Vec3::new(0.375, 0.25, 0.375),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_CHICKEN_BODY; 6],
            pitch_tracks_look: false,
        },
        // Head (face only on front -Z, white sides elsewhere)
        ModelPart {
            origin: Vec3::new(0.0, 0.55, -0.15),
            size: Vec3::new(0.19, 0.19, 0.19),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_CHICKEN_BODY, TEX_CHICKEN_BODY, TEX_CHICKEN_BODY, TEX_CHICKEN_BODY, TEX_CHICKEN_BODY, TEX_CHICKEN_HEAD],
            pitch_tracks_look: false,
        },
        // Left wing
        ModelPart {
            origin: Vec3::new(-0.25, 0.3, 0.0),
            size: Vec3::new(0.0625, 0.2, 0.25),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_CHICKEN_WING; 6],
            pitch_tracks_look: false,
        },
        // Right wing
        ModelPart {
            origin: Vec3::new(0.25, 0.3, 0.0),
            size: Vec3::new(0.0625, 0.2, 0.25),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_CHICKEN_WING; 6],
            pitch_tracks_look: false,
        },
        // Left leg
        ModelPart {
            origin: Vec3::new(-0.06, 0.1, 0.0),
            size: Vec3::new(0.06, 0.2, 0.06),
            pivot: Vec3::new(-0.06, 0.2, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_CHICKEN_LEG; 6],
            pitch_tracks_look: false,
        },
        // Right leg
        ModelPart {
            origin: Vec3::new(0.06, 0.1, 0.0),
            size: Vec3::new(0.06, 0.2, 0.06),
            pivot: Vec3::new(0.06, 0.2, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_CHICKEN_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

/// HP-2 — Bear. Large, low-slung quadruped. Reuses cow body textures
/// (warm brown side, top, end) plus pig leg texture for a darker stub
/// limb. Bigger than a cow, head pushed forward.
fn bear_model() -> Vec<ModelPart> {
    vec![
        // Body — wider + taller than cow.
        ModelPart {
            origin: Vec3::new(0.0, 0.85, 0.0),
            size: Vec3::new(0.85, 0.65, 1.05),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_COW_BODY_SIDE, TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_TOP, TEX_COW_BODY_END, TEX_COW_BODY_END],
            pitch_tracks_look: false,
        },
        // Head — large, pushed forward.
        ModelPart {
            origin: Vec3::new(0.0, 1.1, -0.65),
            size: Vec3::new(0.65, 0.55, 0.5),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_COW_HEAD_SIDE, TEX_COW_HEAD_SIDE, TEX_COW_HEAD_TOP, TEX_COW_HEAD_TOP, TEX_COW_HEAD_FRONT, TEX_COW_HEAD_FRONT],
            pitch_tracks_look: false,
        },
        // Front-left leg
        ModelPart {
            origin: Vec3::new(-0.25, 0.4, -0.35),
            size: Vec3::new(0.25, 0.8, 0.25),
            pivot: Vec3::new(-0.25, 0.8, -0.35),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Front-right leg
        ModelPart {
            origin: Vec3::new(0.25, 0.4, -0.35),
            size: Vec3::new(0.25, 0.8, 0.25),
            pivot: Vec3::new(0.25, 0.8, -0.35),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-left leg
        ModelPart {
            origin: Vec3::new(-0.25, 0.4, 0.35),
            size: Vec3::new(0.25, 0.8, 0.25),
            pivot: Vec3::new(-0.25, 0.8, 0.35),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-right leg
        ModelPart {
            origin: Vec3::new(0.25, 0.4, 0.35),
            size: Vec3::new(0.25, 0.8, 0.25),
            pivot: Vec3::new(0.25, 0.8, 0.35),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_PIG_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

/// HP-2 — Hyena. Lean dog-shaped quadruped. Reuses pig leg + sheep body
/// textures for a sandy hyena coat (Savanna palette).
fn hyena_model() -> Vec<ModelPart> {
    vec![
        // Body — slightly stretched and lean.
        ModelPart {
            origin: Vec3::new(0.0, 0.55, 0.0),
            size: Vec3::new(0.45, 0.45, 0.85),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END, TEX_SHEEP_BODY_END],
            pitch_tracks_look: false,
        },
        // Head — sloped forward with hyena snout.
        ModelPart {
            origin: Vec3::new(0.0, 0.7, -0.5),
            size: Vec3::new(0.35, 0.4, 0.35),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_FRONT, TEX_SHEEP_HEAD_FRONT],
            pitch_tracks_look: false,
        },
        // Front-left leg
        ModelPart {
            origin: Vec3::new(-0.13, 0.25, -0.3),
            size: Vec3::new(0.13, 0.5, 0.13),
            pivot: Vec3::new(-0.13, 0.5, -0.3),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Front-right leg
        ModelPart {
            origin: Vec3::new(0.13, 0.25, -0.3),
            size: Vec3::new(0.13, 0.5, 0.13),
            pivot: Vec3::new(0.13, 0.5, -0.3),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-left leg
        ModelPart {
            origin: Vec3::new(-0.13, 0.25, 0.3),
            size: Vec3::new(0.13, 0.5, 0.13),
            pivot: Vec3::new(-0.13, 0.5, 0.3),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
        // Back-right leg
        ModelPart {
            origin: Vec3::new(0.13, 0.25, 0.3),
            size: Vec3::new(0.13, 0.5, 0.13),
            pivot: Vec3::new(0.13, 0.5, 0.3),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_SHEEP_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

// ── 2026-05-27 mob models ──────────────────────────────────────────
// Seven animals that were spawning but had no model (invisible). Built
// here from box parts the same way Bear/Hyena are, borrowing the
// nearest existing coat texture. Small helpers keep the part lists
// readable; the older models predate these and keep their literal form.

/// `origin`/`size` are the part centre and extent (x, y, z); a static
/// (non-animated) box such as a body, head, ear, or tail.
fn box_part(origin: Vec3, size: Vec3, tex: [u32; 6]) -> ModelPart {
    ModelPart { origin, size, pivot: Vec3::ZERO, animated: false, phase: 0.0, tex_faces: tex, pitch_tracks_look: false }
}

/// An animated leg — pivots at the hip (top-centre of the leg) so the
/// walk cycle swings it, matching the cow/bear/hyena convention.
fn leg_part(origin: Vec3, size: Vec3, phase: f32, tex: u32) -> ModelPart {
    let pivot = Vec3::new(origin.x, origin.y + size.y / 2.0, origin.z);
    ModelPart { origin, size, pivot, animated: true, phase, tex_faces: [tex; 6], pitch_tracks_look: false }
}

/// Face array for a coat: side faces left+right, top+bottom, end+end
/// (front/back). Works for both body and head parts.
fn coat(side: u32, top: u32, end: u32) -> [u32; 6] {
    [side, side, top, top, end, end]
}

/// Horse — tall brown quadruped (cow coat). Long legs, neck, long head.
fn horse_model() -> Vec<ModelPart> {
    let body = coat(TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_END);
    let head = coat(TEX_COW_HEAD_SIDE, TEX_COW_HEAD_TOP, TEX_COW_HEAD_FRONT);
    vec![
        box_part(Vec3::new(0.0, 1.15, 0.05), Vec3::new(0.45, 0.5, 1.1), body),
        box_part(Vec3::new(0.0, 1.5, -0.5), Vec3::new(0.28, 0.5, 0.3), body),   // neck
        box_part(Vec3::new(0.0, 1.7, -0.68), Vec3::new(0.3, 0.32, 0.5), head),  // head
        leg_part(Vec3::new(-0.16, 0.5, -0.4), Vec3::new(0.16, 1.0, 0.16), 0.0, TEX_COW_LEG),
        leg_part(Vec3::new(0.16, 0.5, -0.4), Vec3::new(0.16, 1.0, 0.16), 0.5, TEX_COW_LEG),
        leg_part(Vec3::new(-0.16, 0.5, 0.45), Vec3::new(0.16, 1.0, 0.16), 0.5, TEX_COW_LEG),
        leg_part(Vec3::new(0.16, 0.5, 0.45), Vec3::new(0.16, 1.0, 0.16), 0.0, TEX_COW_LEG),
    ]
}

/// Wolf — lean canine (sheep/cream coat as a stand-in for grey). Pricked
/// ears, snout, bushy tail distinguish it from the Hyena silhouette.
/// TODO: bespoke grey wolf texture in the mob-texture polish pass.
fn wolf_model() -> Vec<ModelPart> {
    let body = coat(TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END);
    let head = coat(TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_FRONT);
    vec![
        box_part(Vec3::new(0.0, 0.62, 0.0), Vec3::new(0.4, 0.42, 0.8), body),
        box_part(Vec3::new(0.0, 0.78, -0.5), Vec3::new(0.32, 0.34, 0.34), head),
        box_part(Vec3::new(0.0, 0.72, -0.72), Vec3::new(0.16, 0.16, 0.2), head),   // snout
        box_part(Vec3::new(-0.1, 1.0, -0.42), Vec3::new(0.08, 0.13, 0.05), head),  // ear L
        box_part(Vec3::new(0.1, 1.0, -0.42), Vec3::new(0.08, 0.13, 0.05), head),   // ear R
        box_part(Vec3::new(0.0, 0.72, 0.5), Vec3::new(0.13, 0.13, 0.3), body),     // tail
        leg_part(Vec3::new(-0.13, 0.25, -0.28), Vec3::new(0.12, 0.5, 0.12), 0.0, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.13, 0.25, -0.28), Vec3::new(0.12, 0.5, 0.12), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(-0.13, 0.25, 0.28), Vec3::new(0.12, 0.5, 0.12), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.13, 0.25, 0.28), Vec3::new(0.12, 0.5, 0.12), 0.0, TEX_SHEEP_LEG),
    ]
}

/// Rabbit — small, with long ears and bigger hind legs (cream coat).
fn rabbit_model() -> Vec<ModelPart> {
    let body = coat(TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END);
    let head = coat(TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_FRONT);
    vec![
        box_part(Vec3::new(0.0, 0.22, 0.05), Vec3::new(0.2, 0.2, 0.34), body),
        box_part(Vec3::new(0.0, 0.3, -0.18), Vec3::new(0.2, 0.2, 0.18), head),
        box_part(Vec3::new(-0.06, 0.5, -0.16), Vec3::new(0.06, 0.22, 0.04), head),  // ear L
        box_part(Vec3::new(0.06, 0.5, -0.16), Vec3::new(0.06, 0.22, 0.04), head),   // ear R
        box_part(Vec3::new(0.0, 0.24, 0.24), Vec3::new(0.09, 0.09, 0.06), body),    // tail
        leg_part(Vec3::new(-0.07, 0.09, -0.12), Vec3::new(0.07, 0.18, 0.07), 0.0, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.07, 0.09, -0.12), Vec3::new(0.07, 0.18, 0.07), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(-0.08, 0.1, 0.16), Vec3::new(0.09, 0.2, 0.12), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.08, 0.1, 0.16), Vec3::new(0.09, 0.2, 0.12), 0.0, TEX_SHEEP_LEG),
    ]
}

/// Goat — sheep-shaped (cream coat) with two back-swept horns + a beard.
fn goat_model() -> Vec<ModelPart> {
    let body = coat(TEX_SHEEP_BODY_SIDE, TEX_SHEEP_BODY_TOP, TEX_SHEEP_BODY_END);
    let head = coat(TEX_SHEEP_HEAD_SIDE, TEX_SHEEP_HEAD_TOP, TEX_SHEEP_HEAD_FRONT);
    vec![
        box_part(Vec3::new(0.0, 0.6, 0.0), Vec3::new(0.4, 0.42, 0.7), body),
        box_part(Vec3::new(0.0, 0.72, -0.45), Vec3::new(0.3, 0.3, 0.32), head),
        box_part(Vec3::new(-0.1, 0.95, -0.36), Vec3::new(0.06, 0.2, 0.06), head),   // horn L
        box_part(Vec3::new(0.1, 0.95, -0.36), Vec3::new(0.06, 0.2, 0.06), head),    // horn R
        box_part(Vec3::new(0.0, 0.55, -0.5), Vec3::new(0.06, 0.13, 0.05), head),    // beard
        leg_part(Vec3::new(-0.13, 0.25, -0.25), Vec3::new(0.11, 0.5, 0.11), 0.0, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.13, 0.25, -0.25), Vec3::new(0.11, 0.5, 0.11), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(-0.13, 0.25, 0.25), Vec3::new(0.11, 0.5, 0.11), 0.5, TEX_SHEEP_LEG),
        leg_part(Vec3::new(0.13, 0.25, 0.25), Vec3::new(0.11, 0.5, 0.11), 0.0, TEX_SHEEP_LEG),
    ]
}

/// Nostrich — tall two-legged bird (brown cow coat). Long neck + tail
/// tuft; only two legs, so it reads as a flightless ratite.
fn nostrich_model() -> Vec<ModelPart> {
    let body = coat(TEX_COW_BODY_SIDE, TEX_COW_BODY_TOP, TEX_COW_BODY_END);
    let head = coat(TEX_COW_HEAD_SIDE, TEX_COW_HEAD_TOP, TEX_COW_HEAD_FRONT);
    vec![
        box_part(Vec3::new(0.0, 0.95, 0.05), Vec3::new(0.42, 0.5, 0.62), body),
        box_part(Vec3::new(0.0, 1.4, -0.18), Vec3::new(0.16, 0.62, 0.16), body),    // neck
        box_part(Vec3::new(0.0, 1.85, -0.24), Vec3::new(0.2, 0.2, 0.3), head),      // head
        box_part(Vec3::new(0.0, 1.82, -0.44), Vec3::new(0.08, 0.08, 0.14), head),   // beak
        box_part(Vec3::new(0.0, 1.0, 0.42), Vec3::new(0.32, 0.28, 0.22), body),     // tail tuft
        leg_part(Vec3::new(-0.12, 0.45, 0.05), Vec3::new(0.1, 0.9, 0.1), 0.0, TEX_COW_LEG),
        leg_part(Vec3::new(0.12, 0.45, 0.05), Vec3::new(0.1, 0.9, 0.1), 0.5, TEX_COW_LEG),
    ]
}

/// Bee — small yellow/black-striped flyer. Wings, no legs.
fn bee_model() -> Vec<ModelPart> {
    let body = [TEX_BEE_BODY; 6];
    vec![
        box_part(Vec3::new(0.0, 0.55, 0.0), Vec3::new(0.3, 0.3, 0.4), body),
        box_part(Vec3::new(0.0, 0.55, -0.28), Vec3::new(0.2, 0.2, 0.16), [TEX_BEE_HEAD; 6]),
        box_part(Vec3::new(-0.13, 0.73, 0.0), Vec3::new(0.02, 0.04, 0.28), [TEX_CHICKEN_WING; 6]), // wing L (pale, translucent read)
        box_part(Vec3::new(0.13, 0.73, 0.0), Vec3::new(0.02, 0.04, 0.28), [TEX_CHICKEN_WING; 6]),  // wing R
        box_part(Vec3::new(0.0, 0.55, 0.24), Vec3::new(0.05, 0.05, 0.1), [TEX_BEE_HEAD; 6]),        // stinger
    ]
}

/// Squid — aquatic; ocean-blue pointed mantle over four hanging tentacles.
fn squid_model() -> Vec<ModelPart> {
    let body = [TEX_SQUID_BODY; 6];
    vec![
        box_part(Vec3::new(0.0, 0.7, 0.0), Vec3::new(0.42, 0.5, 0.42), body),       // mantle
        box_part(Vec3::new(0.0, 0.42, 0.0), Vec3::new(0.44, 0.18, 0.44), body),     // eyes band
        box_part(Vec3::new(-0.13, 0.18, -0.13), Vec3::new(0.08, 0.4, 0.08), body),
        box_part(Vec3::new(0.13, 0.18, -0.13), Vec3::new(0.08, 0.4, 0.08), body),
        box_part(Vec3::new(-0.13, 0.18, 0.13), Vec3::new(0.08, 0.4, 0.08), body),
        box_part(Vec3::new(0.13, 0.18, 0.13), Vec3::new(0.08, 0.4, 0.08), body),
    ]
}

/// Aquatic wave — a small streamlined fish: a flattened body long in +z with a
/// little tail fin. Reuses the squid body texture (silvery-blue). Polish pass
/// gives it a dedicated scale texture.
fn fish_model() -> Vec<ModelPart> {
    let body = [TEX_SQUID_BODY; 6];
    vec![
        box_part(Vec3::new(0.0, 0.15, 0.05), Vec3::new(0.10, 0.11, 0.20), body), // body
        box_part(Vec3::new(0.0, 0.15, -0.20), Vec3::new(0.03, 0.13, 0.08), body), // tail fin
    ]
}

/// Aquatic wave — the Shark: a big body long in z, a dorsal fin, and a tall
/// tail. Reuses the squid texture for v1 (a grey shark skin is a polish pass).
fn shark_model() -> Vec<ModelPart> {
    let body = [TEX_SQUID_BODY; 6];
    vec![
        box_part(Vec3::new(0.0, 0.45, 0.10), Vec3::new(0.28, 0.30, 0.62), body),  // body
        box_part(Vec3::new(0.0, 0.85, 0.05), Vec3::new(0.05, 0.22, 0.18), body),  // dorsal fin
        box_part(Vec3::new(0.0, 0.45, -0.62), Vec3::new(0.06, 0.34, 0.14), body), // tail
    ]
}

/// Campaign N (2026-07-05) — overwrite freshly built verts with a sampled
/// world-light pair `(block/15, sky/15)` so the entity pipeline's `fs_main`
/// night formula applies to entities exactly as it does to terrain. One
/// sample per entity (the Minecraft-parity look). Emissive verts carrying a
/// `> 1.5` sentinel (Satoshi's amulet glow) are preserved.
pub(crate) fn apply_light_channels(verts: &mut [Vertex], light: (f32, f32)) {
    for v in verts {
        if v.light <= 1.5 {
            v.light = light.0;
            v.sky_light = light.1;
        }
    }
}

/// Campaign N — avatar-pipeline verts carry a CPU-resolved combined light in
/// `sky_light` (their `light` channel is the Phase-3 fade alpha, untouchable);
/// `fs_avatar` multiplies rgb by `max(sky_light, 0.08)`. `push_skin_quad`
/// defaults it to 1.0, so preview/workshop feeders stay bright unless a call
/// site dims them explicitly.
pub(crate) fn set_avatar_light(verts: &mut [Vertex], combined: f32) {
    for v in verts {
        v.sky_light = combined;
    }
}

/// Resolve the avatar combined-light scalar from sampled `(block, sky)`
/// channels + the frame's sky brightness — the same `max(block, sky * sun.w)`
/// formula `fs_main` runs on the GPU for entities.
pub(crate) fn combined_light(channels: (f32, f32), sky_brightness: f32) -> f32 {
    channels.0.max(channels.1 * sky_brightness)
}

/// Build textured vertices for all entities. Uses the chunk Vertex format so
/// entities render with the same pipeline (lighting, fog, textures).
///
/// `overrides` (Spec 40 — The Workshop) supplies any per-mob-part appearance
/// reskins; a part with no override renders byte-identically.
///
/// `light_at` samples world light at an entity's feet position
/// (`World::light_channels_at`); pass `|_| (1.0, 0.0)` for full-bright
/// previews.
pub fn build_entity_model_vertices(
    ecs: &hecs::World,
    time_ticks: u32,
    player_pos: Vec3,
    overrides: &crate::override_registry::OverrideRegistry,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    let anim_time = time_ticks as f32 / 20.0; // seconds

    for (_id, (pos, vel, kind, _hitbox, ai, health, baby, satoshi)) in ecs
        .query::<(
            &Position,
            &Velocity,
            &MobKind,
            &Hitbox,
            &MobAi,
            &Health,
            Option<&crate::breeding::Baby>,
            Option<&crate::satoshi::SatoshiMarker>,
        )>()
        .iter()
    {
        let is_satoshi = satoshi.is_some();
        // Skip entities too close to camera to avoid seeing insides
        let dist_sq = (pos.0 - player_pos).length_squared();
        if dist_sq < 0.25 {
            continue;
        }
        // P5 — babies render at a reduced scale so juveniles read as juveniles.
        let model_scale = if baby.is_some() {
            crate::breeding::BABY_RENDER_SCALE
        } else {
            1.0
        };
        let model: &[ModelPart] =
            if is_satoshi { SATOSHI_MODEL.as_slice() } else { mob_model(kind.0) };
        let flashing = health.is_flashing();
        let speed = (vel.0.x * vel.0.x + vel.0.z * vel.0.z).sqrt();
        // Use AI facing direction (persists when idle) + π/2 offset because
        // model front is -Z but ai.facing=0 means +X movement.
        let yaw = ai.facing + std::f32::consts::FRAC_PI_2;

        let verts_start = verts.len();
        for (part_idx, part) in model.iter().enumerate() {
            let swing = if part.animated && speed > 0.001 {
                let freq = 2.5; // leg swing frequency
                (anim_time * freq + part.phase * std::f32::consts::TAU).sin() * 0.4
            } else {
                0.0
            };

            // Spec 40 — resolve this part's reskin (if any) over its defaults.
            // Satoshi uses his own fixed faces — never a villager workshop reskin.
            let reskin = if is_satoshi {
                None
            } else {
                overrides.mob_part_faces(kind.0, part_idx as u8, &part.tex_faces)
            };

            let part_start = verts.len();
            build_part_vertices(
                &mut verts,
                part,
                pos.0,
                yaw,
                PartPose { walk_swing: swing, head_pitch: 0.0, arm_override: None, ..Default::default() },
                reskin.as_ref(),
                model_scale,
            );
            // Satoshi's amulet self-glows: stamp the emissive sentinel onto its
            // vertices so the shader renders them unlit (a warm glow in any light).
            if is_satoshi && satoshi_emissive_part(part_idx) {
                for v in &mut verts[part_start..] {
                    v.light = SATOSHI_GLOW_LIGHT;
                }
            }
        }

        // Damage flash: override normals to point straight up for maximum brightness,
        // making the entity appear to "flash" bright white when damaged
        if flashing {
            for v in &mut verts[verts_start..] {
                v.normal = [0.0, 1.0, 0.0]; // All faces max brightness
            }
        }

        // Campaign N — mobs dim with the world's light. The GlowSquid is the
        // game's living lamp and stays full-bright by design.
        if kind.0 != MobType::GlowSquid {
            apply_light_channels(&mut verts[verts_start..], light_at(pos.0));
        }
    }

    verts
}

/// Spec 40 (The Workshop) — render a mob "mannequin": a still, posable preview at
/// `pos`, uniformly `scale`d (the bellows inflation), reskinned by `overrides`, and
/// animated with the walk cycle only when `play`. Used for the in-world inflated
/// working copy of a mob redesign project.
pub fn build_mob_mannequin_vertices(
    mob: MobType,
    pos: Vec3,
    yaw: f32,
    scale: f32,
    play: bool,
    anim_time: f32,
    overrides: &crate::override_registry::OverrideRegistry,
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    for (part_idx, part) in mob_model(mob).iter().enumerate() {
        let swing = if play && part.animated {
            (anim_time * 2.5 + part.phase * std::f32::consts::TAU).sin() * 0.4
        } else {
            0.0
        };
        let reskin = overrides.mob_part_faces(mob, part_idx as u8, &part.tex_faces);
        build_part_vertices(
            &mut verts,
            part,
            pos,
            yaw,
            PartPose { walk_swing: swing, head_pitch: 0.0, arm_override: None, ..Default::default() },
            reskin.as_ref(),
            scale,
        );
    }
    verts
}

/// Spec 40 (The Workshop) — render a single block as an inflated cube mannequin at
/// `pos` (centre), edge length `scale`, with any reskin applied per face. Used for
/// the in-world inflated working copy of a block redesign project.
pub fn build_block_mannequin_vertices(
    block_id: crate::block::BlockId,
    pos: Vec3,
    scale: f32,
    registry: &crate::block::BlockRegistry,
    overrides: &crate::override_registry::OverrideRegistry,
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    let h = scale * 0.5;
    let c = pos;
    let corners = [
        Vec3::new(c.x - h, c.y - h, c.z - h), // 0 ---
        Vec3::new(c.x + h, c.y - h, c.z - h), // 1 +--
        Vec3::new(c.x + h, c.y + h, c.z - h), // 2 ++-
        Vec3::new(c.x - h, c.y + h, c.z - h), // 3 -+-
        Vec3::new(c.x - h, c.y - h, c.z + h), // 4 --+
        Vec3::new(c.x + h, c.y - h, c.z + h), // 5 +-+
        Vec3::new(c.x + h, c.y + h, c.z + h), // 6 +++
        Vec3::new(c.x - h, c.y + h, c.z + h), // 7 -++
    ];
    // Per-face layer: override (keyed by mesh::Face::index()) else the default
    // top/bottom/side. Face::index(): Top=0, Bottom=1, North=2, South=3, East=4, West=5.
    let top = registry.tex_top(block_id);
    let bottom = registry.tex_bottom(block_id);
    let side = registry.tex_side(block_id);
    let layer = |face_index: u8, default: u32| {
        overrides.block_face_layer(block_id, face_index).unwrap_or(default)
    };
    // +X (East=4)
    push_textured_quad(&mut verts, layer(4, side), [1.0, 0.0, 0.0], corners[1], corners[5], corners[6], corners[2]);
    // -X (West=5)
    push_textured_quad(&mut verts, layer(5, side), [-1.0, 0.0, 0.0], corners[4], corners[0], corners[3], corners[7]);
    // +Y (Top=0)
    push_textured_quad(&mut verts, layer(0, top), [0.0, 1.0, 0.0], corners[3], corners[2], corners[6], corners[7]);
    // -Y (Bottom=1)
    push_textured_quad(&mut verts, layer(1, bottom), [0.0, -1.0, 0.0], corners[4], corners[5], corners[1], corners[0]);
    // +Z (South=3)
    push_textured_quad(&mut verts, layer(3, side), [0.0, 0.0, 1.0], corners[5], corners[4], corners[7], corners[6]);
    // -Z (North=2)
    push_textured_quad(&mut verts, layer(2, side), [0.0, 0.0, -1.0], corners[0], corners[1], corners[2], corners[3]);
    verts
}

/// Spec 40 — render every Workshop project as an in-world inflated mannequin (the
/// "blow it up to work on it" view). Each project sits at its placed origin, scaled
/// by its bellows inflation, reskinned by the override registry; mob projects
/// animate when `play`. Pinned projects are skipped (they're committed). Returns
/// chunk-format vertices for the entity pass.
pub fn build_workshop_inworld_vertices(
    world: &crate::world::World,
    registry: &crate::block::BlockRegistry,
    anim_time: f32,
    play: bool,
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    for project in world.workshop.iter() {
        if !project.is_parked() {
            continue; // pinned — already applied to every instance
        }
        if matches!(project.target, crate::workshop::WorkshopTarget::Avatar) {
            continue; // avatar renders through the skin pipeline (avatar_verts)
        }
        // Spec 40 blow-up Phase 2 — an animated working copy grows from the AIMED
        // block (the cage's near corner) UP and AWAY toward the far corner, so it
        // sits on the block at ×1 and exactly fills the green cage at ×4. Non-
        // blow-up projects (e.g. `/ws place` mannequins) fall through to the
        // floating-centred placement below, unchanged. Blocks only (mob blow-up
        // is out of v1).
        if let Some(b) = project.blow_up {
            // Phase 3 — a locked, edited working copy renders from its 16³ edit
            // buffer (paint + sculpt visible), baked through the micro-model
            // mesher and scaled ×4 into the cage. Falls through to the Phase-2
            // animated cube while charging/collapsing or before the first edit.
            if b.phase == crate::workshop::BlowUpPhase::Locked
                && let Some(buf) = &project.edit {
                    let mm = buf.to_micro_model();
                    let mesh = crate::micro_model::bake_micro_model(&mm, registry);
                    let c = b.corner; // cage min corner
                    let cf = [c[0] as f32, c[1] as f32, c[2] as f32];
                    const SPAN: f32 = 4.0; // model [0,1] → cage [corner, corner+4]
                    for &idx in &mesh.indices {
                        let mut v = mesh.vertices[idx as usize];
                        v.position = [
                            cf[0] + v.position[0] * SPAN,
                            cf[1] + v.position[1] * SPAN,
                            cf[2] + v.position[2] * SPAN,
                        ];
                        verts.push(v);
                    }
                    continue;
                }
            if let crate::workshop::WorkshopTarget::Block(id) = &project.target {
                let s = b.scale();
                let o = project.origin; // the aimed block (the near corner)
                let c = b.corner; // cage min corner (= blow_up_cage_min at charge start)
                // build_block_mannequin_vertices centres a cube of edge `s` at `pos`.
                // Per axis: if the cage min == the block, the block is the min/near
                // corner and the cube grows toward +axis from the block's low face
                // (centre = block + s/2). Otherwise the block is the max/near corner
                // and the cube grows toward −axis from the block's high face
                // (centre = (block+1) − s/2). Both meet the cage exactly at s=4 and
                // sit on the block at s=1.
                let axis = |bk: i32, mn: i32| -> f32 {
                    if mn == bk {
                        bk as f32 + s * 0.5
                    } else {
                        (bk + 1) as f32 - s * 0.5
                    }
                };
                let base = Vec3::new(axis(o[0], c[0]), axis(o[1], c[1]), axis(o[2], c[2]));
                verts.extend(build_block_mannequin_vertices(
                    *id,
                    base,
                    s,
                    registry,
                    &world.override_registry,
                ));
            }
            continue;
        }
        let scale = crate::workshop::inflation_scale(project.inflation);
        let o = project.origin;
        // Sit the mannequin a little above the placed anchor so it reads as a
        // floating working copy; centre blocks, feet mobs.
        let base = Vec3::new(o[0] as f32 + 0.5, o[1] as f32 + 1.0 + scale * 0.5, o[2] as f32 + 0.5);
        match &project.target {
            crate::workshop::WorkshopTarget::Block(id) => {
                verts.extend(build_block_mannequin_vertices(
                    *id,
                    base,
                    scale,
                    registry,
                    &world.override_registry,
                ));
            }
            crate::workshop::WorkshopTarget::Mob(mob) => {
                let feet = Vec3::new(o[0] as f32 + 0.5, o[1] as f32 + 1.0, o[2] as f32 + 0.5);
                verts.extend(build_mob_mannequin_vertices(
                    *mob,
                    feet,
                    0.0,
                    scale,
                    play,
                    anim_time,
                    &world.override_registry,
                ));
            }
            // Unreachable after the top-of-loop guard — the avatar renders through
            // the skin pipeline (`build_workshop_avatar_vertices`). Kept so the
            // match stays total.
            crate::workshop::WorkshopTarget::Avatar => {}
        }
    }
    verts
}

/// Skin-pipeline vertices for the Workshop's blown-up AVATAR working copy, if one
/// is parked. Renders the equipped skin (array layer 0, painted live) at the
/// blow-up scale, anchored at `WORKSHOP_MANNEQUIN_POS`. Returns empty if no
/// avatar project. Caller appends to `avatar_verts` (the avatar pipeline), NOT
/// `entity_verts`. The held mesh is dropped (the mannequin holds nothing).
pub fn build_workshop_avatar_vertices(
    world: &crate::world::World,
    registry: &crate::block::BlockRegistry,
    opts: SkinRenderOpts,
) -> Vec<Vertex> {
    for project in world.workshop.iter() {
        if !matches!(project.target, crate::workshop::WorkshopTarget::Avatar) {
            continue;
        }
        let Some(b) = project.blow_up else { continue };
        // Charge/collapse animate the scale 1→AVATAR_BLOW_UP_SCALE; Locked sits at
        // full. Reuse the block balloon's eased scale, remapped to the avatar's
        // visual scale (b.scale() is 1..=4 for the block ×4 cage).
        let frac = (b.scale() - 1.0) / 3.0; // 0..1
        let s = 1.0 + frac * (crate::workshop::AVATAR_BLOW_UP_SCALE - 1.0);
        let pos = crate::workshop::WORKSHOP_MANNEQUIN_POS;
        // Build the mannequin in MODEL space, then apply the one shared
        // model→world transform. The part builder pre-rotates by `yaw + π/2`
        // (see `build_player_avatar_vertices_with`), so a model yaw of −π/2 makes
        // that rotation the identity and a zero position leaves every vertex
        // exactly where `skin_pose::part_box` says it is — the space the paint
        // hit-test and the paint aids work in.
        let ps = crate::protocol::PlayerState {
            player_index: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            health: 20.0,
            held_kind: 0,
            held_id: 0,
            anim_state: 0,
            flags: 0,
            skin_key: 0,
        };
        let (mut skin_v, _held) =
            build_player_avatar_vertices_with(&ps, 0.0, 0, 0 /* layer 0 */, 1.0, registry, opts);
        // ONE transform, shared: `skin_grid::avatar_model_to_world` is the exact
        // inverse of `workshop::world_ray_to_avatar_model` (the paint ray) and is
        // what the grid / hover-footprint aids draw with. Rotate about Y by
        // `yaw + π/2`, scale about the feet anchor, translate. Round-trip tests
        // in `skin_grid` guard the pairing; never re-derive it here.
        for v in skin_v.iter_mut() {
            v.position = crate::skin_grid::avatar_model_to_world(
                v.position,
                pos,
                crate::workshop::WORKSHOP_MANNEQUIN_YAW,
                s,
            );
        }
        return skin_v;
    }
    Vec::new()
}

/// Build the textured mesh for one remote player avatar (Phase 6 / P1-T4).
/// Mirrors `build_entity_model_vertices` for a single humanoid: walk-cycle
/// swing on arms/legs when walking, head pitch from look, a swing override on
/// the right arm during a mine/place swing, an optional crouch dip, and the
/// held item anchored to the right hand. Local-space part geometry comes from
/// `player_model()`; the wire `PlayerState` is the single source of truth.
///
/// Returns `(skin_verts, held_verts)`:
/// - **`skin_verts`** — the avatar body, box-unwrapped onto the 64x64 skin
///   atlas. Each of the 6 parts emits TWO boxes: the solid **base** layer
///   (`base_faces()`, inflate 0.0) and the **overlay** hat/jacket/sleeve layer
///   (`overlay_faces()`, inflate `OVERLAY_INFLATE` so it sits just outside the
///   base and the alpha-cutout shader can show skin through transparent overlay
///   pixels without z-fighting). Rendered through the renderer's
///   `avatar_pipeline` against the dedicated skin texture.
/// - **`held_verts`** — the held-item sub-mesh, block/atlas-textured. Rendered
///   through the ordinary `entity_pipeline` (the block texture array) because it
///   is a world block, not part of the skin.
///   World x-z direction the avatar's FRONT (face) points, for a model yaw `phi`.
///   Mirrors [`build_skin_part_vertices`]: the **-Z** cube face is the front, and
///   the body is rotated about Y by `phi + π/2`. So the local front `(0,-1)` maps
///   to `(sin(phi+π/2), -cos(phi+π/2)) == (cos phi, sin phi)`. Pure + tested so the
///   facing convention is regression-guarded (a builder change must update both).
///
/// No production caller — workshop.rs references the formula in a comment but
/// doesn't call this directly. Tested in this module.
#[cfg_attr(not(test), allow(dead_code))]
pub fn avatar_front_dir(phi: f32) -> (f32, f32) {
    let y = phi + std::f32::consts::FRAC_PI_2;
    (y.sin(), -y.cos())
}

/// The model yaw that makes the avatar FACE horizontal direction `(dx, dz)` —
/// the inverse of [`avatar_front_dir`]. Used to point the third-person
/// self-avatar where the camera looks, so you see the back of the head (and it
/// tracks the look, not counter-rotates). NB the engine's *camera* yaw uses a
/// different convention (`forward = (-sin, -cos)`), which is exactly why this
/// conversion is needed rather than passing `camera.yaw` straight through.
pub fn yaw_facing(dx: f32, dz: f32) -> f32 {
    dz.atan2(dx)
}

/// Vertices for the LOCAL player's OWN avatar, gated by camera mode. In
/// first-person the player sees the viewmodel hand instead, so this returns
/// `(empty, empty)`; in any third-person mode it returns the same body + held
/// mesh a remote peer would draw via [`build_player_avatar_vertices`]. Pure and
/// GPU-free, so the "your body shows in third-person, hides in first-person" rule
/// is unit-testable without a renderer. Cross-platform (drawn on the PWA target
/// too). Spec: `docs/foundations/2026-06-09-third-person-camera.md` (Phase 1).
#[allow(clippy::too_many_arguments)]
pub fn self_avatar_vertices(
    mode: crate::camera::CameraMode,
    ps: &crate::protocol::PlayerState,
    swing_progress: f32,
    time_ticks: u32,
    skin_layer: u32,
    // Phase 3 — self-avatar fade alpha [0,1] baked into the skin verts' light
    // channel for the `fs_avatar` screen-door dither. 1.0 = fully opaque.
    fade: f32,
    registry: &crate::block::BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
) -> (Vec<Vertex>, Vec<Vertex>) {
    if mode.is_third_person() {
        build_player_avatar_vertices(
            ps,
            swing_progress,
            time_ticks,
            skin_layer,
            fade,
            registry,
            arm_model,
        )
    } else {
        (Vec::new(), Vec::new())
    }
}

pub fn build_player_avatar_vertices_with(
    ps: &crate::protocol::PlayerState,
    swing_progress: f32, // 0.0..=1.0 within the swing window; 0 = not swinging
    time_ticks: u32,
    skin_layer: u32,
    // Phase 3 — fade alpha [0,1] for the SKIN verts (the avatar pipeline reads it
    // as a dither alpha). Remote/split-screen/preview callers pass 1.0; only the
    // local self-avatar fades when the camera pulls in. Held-item verts go to the
    // entity pipeline and are never faded.
    fade: f32,
    registry: &crate::block::BlockRegistry,
    opts: SkinRenderOpts,
) -> (Vec<Vertex>, Vec<Vertex>) {
    use crate::protocol::player_flags;
    let mut skin_verts = Vec::new();
    let mut held_verts = Vec::new();
    let anim_time = time_ticks as f32 / 20.0; // seconds
    let yaw = ps.yaw + std::f32::consts::FRAC_PI_2; // match mob facing convention

    // Crouch dip: lower the whole avatar by a small constant.
    let crouch_dip = if ps.flags & player_flags::CROUCHING != 0 { 0.25 } else { 0.0 };
    let foot_pos = Vec3::new(ps.x, ps.y - crouch_dip, ps.z);

    // The right-arm swing angle (if mid-swing) — reused for both the arm part
    // and the held-item transform so the item tracks the hand.
    let arm_swing = if swing_progress > 0.0 { Some(swing_angle(swing_progress)) } else { None };

    // Box-unwrap UV tables — owned arrays; compute once, index by part i.
    let base_uv = crate::skin_uv::base_faces(opts.arm_model);
    let overlay_uv = crate::skin_uv::overlay_faces(opts.arm_model);
    let parts = player_model_for(opts.arm_model);

    const LEFT_ARM_INDEX: usize = 2;
    const RIGHT_ARM_INDEX: usize = 3;
    const LEFT_LEG_INDEX: usize = 4;
    const RIGHT_LEG_INDEX: usize = 5;
    for (i, part) in parts.iter().enumerate() {
        let walk_swing = if ps.anim_state == 1 && part.animated {
            // Walk: alternating sinusoidal swing per part phase.
            (anim_time * 2.5 + part.phase * std::f32::consts::TAU).sin() * 0.4
        } else if ps.anim_state == 2 && part.animated {
            // Jump: static airborne pose — legs tuck back, arms lift forward/up,
            // so an airborne avatar reads as jumping rather than idle. Values are
            // playtest-tunable; the right arm's swing (arm_override) still wins
            // over this when SWINGING.
            match i {
                LEFT_LEG_INDEX | RIGHT_LEG_INDEX => 0.5, // legs tuck behind
                LEFT_ARM_INDEX | RIGHT_ARM_INDEX => -0.4, // arms forward/up
                _ => 0.0,
            }
        } else {
            0.0
        };
        let arm_override = if i == RIGHT_ARM_INDEX { arm_swing } else { None };
        let pose = PartPose { walk_swing, head_pitch: ps.pitch, arm_override, ..Default::default() };
        // Limbs-apart displaces the part in MODEL space. The part builder applies
        // R_y(yaw) then translates by entity_pos, so the offset must be rotated
        // by the same yaw to stay a model-space displacement — otherwise the
        // limbs would separate along world axes while the hit-test (which works
        // purely in model space, via `world_ray_to_avatar_model`) expects them to
        // separate along the avatar's own. `skin_pose` is the shared source.
        let off = crate::skin_pose::part_offset(i, opts.separation);
        let (cos_y, sin_y) = (yaw.cos(), yaw.sin());
        let part_pos = foot_pos
            + Vec3::new(
                off[0] * cos_y - off[2] * sin_y,
                off[1],
                off[0] * sin_y + off[2] * cos_y,
            );
        // Base body layer (solid).
        build_skin_part_vertices(&mut skin_verts, part, &base_uv[i], 0.0, part_pos, yaw, pose, skin_layer, fade);
        // Overlay layer (hat/jacket/sleeve/leg2), inflated just outside the base.
        if opts.draw_overlay {
            let inflate = if opts.edit_inflate {
                crate::skin_pose::EDIT_INFLATE
            } else {
                crate::skin_pose::overlay_inflate(i)
            };
            build_skin_part_vertices(&mut skin_verts, part, &overlay_uv[i], inflate, part_pos, yaw, pose, skin_layer, fade);
        }
    }

    // Held item: anchor to the right hand. The right arm (player_model[3])
    // has pivot (0.375, 1.4, 0.0) and its hand-end sits at the bottom of the
    // arm cuboid (origin.y - size.y/2 = 1.05 - 0.35 = 0.70). We build the
    // local-space item mesh centred at the hand, rotate by the arm swing
    // about the arm pivot (X), then by yaw (Y), then translate to foot_pos —
    // matching the part-builder transform order so the item rides the hand.
    let item = crate::protocol::ItemRef::from_wire(ps.held_kind, ps.held_id);
    if !matches!(item, crate::protocol::ItemRef::Empty) {
        // Derived from the right-arm part rather than hand-typed, so a slim
        // arm's hand (narrower and half a pixel lower) still holds the item.
        // For classic these evaluate to the historical (0.375, 0.70) / (0.375, 1.4).
        let right_arm = &parts[RIGHT_ARM_INDEX];
        let hand_anchor = Vec3::new(
            right_arm.origin.x,
            right_arm.origin.y - right_arm.size.y * 0.5,
            right_arm.origin.z,
        );
        let arm_pivot = right_arm.pivot;
        let mut item_verts = crate::held_item_model::held_item_mesh(item, registry);
        let x_rot = arm_swing.unwrap_or(0.0);
        let (sin_x, cos_x) = x_rot.sin_cos();
        let (sin_y, cos_y) = yaw.sin_cos();
        for v in &mut item_verts {
            // Translate local-space item to the hand anchor.
            let mut p = Vec3::from(v.position) + hand_anchor;
            // Arm swing about the arm pivot (X axis), matching build_part_vertices.
            if x_rot.abs() > 0.001 {
                let dy = p.y - arm_pivot.y;
                let dz = p.z - arm_pivot.z;
                p.y = arm_pivot.y + dy * cos_x - dz * sin_x;
                p.z = arm_pivot.z + dy * sin_x + dz * cos_x;
            }
            // Yaw about Y, then translate to foot position.
            let rx = p.x * cos_y - p.z * sin_y;
            let rz = p.x * sin_y + p.z * cos_y;
            v.position = [rx + foot_pos.x, p.y + foot_pos.y, rz + foot_pos.z];
        }
        held_verts.extend(item_verts);
    }

    // ARMOUR HOOK: overlay armour-layer meshes here when armour rendering lands (design §3 non-goal).

    (skin_verts, held_verts)
}

/// The WORN avatar — parts at rest, Minecraft-exact overlay inflate, clothes on.
/// Thin wrapper over [`build_player_avatar_vertices_with`]; every remote-player,
/// split-screen and thumbnail caller goes through here.
#[allow(clippy::too_many_arguments)]
pub fn build_player_avatar_vertices(
    ps: &crate::protocol::PlayerState,
    swing_progress: f32,
    time_ticks: u32,
    skin_layer: u32,
    fade: f32,
    registry: &crate::block::BlockRegistry,
    arm_model: crate::skin_uv::ArmModel,
) -> (Vec<Vertex>, Vec<Vertex>) {
    build_player_avatar_vertices_with(
        ps,
        swing_progress,
        time_ticks,
        skin_layer,
        fade,
        registry,
        SkinRenderOpts { arm_model, ..Default::default() },
    )
}

/// Build vertices for every dropped-item entity. Each item becomes a small
/// floating cube that bobs gently on Y and slowly spins on Y so it reads as
/// "loot" at a glance. Block items use their block's textures; other items
/// (materials, tools) currently fall back to a generic plank texture — Wave
/// 2b polish pass adds per-material textures + a per-tool one.
pub fn build_item_entity_vertices(
    ecs: &hecs::World,
    registry: &crate::block::BlockRegistry,
    time_ticks: u32,
    player_pos: Vec3,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    let anim_time = time_ticks as f32 / 20.0;

    for (_id, (pos, item)) in ecs.query::<(&Position, &ItemEntity)>().iter() {
        // Skip if too close to camera (avoid seeing the inside).
        if (pos.0 - player_pos).length_squared() < 0.05 {
            continue;
        }
        let (tex_top, tex_side, tex_bottom) = item_textures(&item.stack.item, registry);

        push_item_drop_cube(
            &mut verts,
            (tex_top, tex_side, tex_bottom),
            pos.0,
            anim_time,
            light_at(pos.0),
        );
    }

    verts
}

/// Drop-cube textures for one inventory `Item`. Material/tool drops use the
/// dedicated `TEX_ITEM_*` layers; block drops use the block's own textures.
/// Shared by the local `ItemEntity` pass and the remote-item pass, so a
/// server-broadcast tool/armour drop looks identical to a local one
/// (death-drops phase 3).
fn item_textures(item: &Item, registry: &crate::block::BlockRegistry) -> (u32, u32, u32) {
    match item {
        Item::Block(id) => {
            let def = registry.get(*id);
            (def.tex_top, def.tex_side, def.tex_bottom)
        }
        Item::Material(mid) => {
            let tex = material_texture(*mid);
            (tex, tex, tex)
        }
        Item::Tool(t) => {
            let tex = tool_texture(t.material);
            (tex, tex, tex)
        }
        Item::Plan(_) => {
            // Spec 24 — ground-dropped Plan item. Render as a
            // small parchment scroll cube using the Blueprint Paper top
            // texture for the parchment face + the plaque side
            // texture for the wood-rolled-edge sides.
            (
                crate::block::TEX_BLUEPRINT_PAPER_TOP,
                crate::block::TEX_ARCHITECT_PLAQUE_SIDE,
                crate::block::TEX_BLUEPRINT_PAPER_TOP,
            )
        }
        Item::Armour(_) => {
            // Spec 28e — ground-dropped armour. BRIDGE: reuse iron-
            // ingot texture for all tiers; per-slot icons land with
            // the live armour HUD work.
            (TEX_ITEM_IRON_INGOT, TEX_ITEM_IRON_INGOT, TEX_ITEM_IRON_INGOT)
        }
    }
}

/// Emit one bobbing, spinning drop cube — the shared "loot at a glance" mesh
/// for local `ItemEntity`s and server-broadcast remote items (phase 2b).
fn push_item_drop_cube(
    verts: &mut Vec<Vertex>,
    (tex_top, tex_side, tex_bottom): (u32, u32, u32),
    pos: Vec3,
    anim_time: f32,
    light: (f32, f32),
) {
    let verts_start = verts.len();
    let bob = (anim_time * 2.0).sin() * 0.06;
    let yaw = anim_time * 0.9;
    let centre = pos + Vec3::new(0.0, 0.18 + bob, 0.0);

    let size = 0.2;
    let half = size * 0.5;
    let mut corners = [
        Vec3::new(-half, -half, -half),
        Vec3::new( half, -half, -half),
        Vec3::new( half,  half, -half),
        Vec3::new(-half,  half, -half),
        Vec3::new(-half, -half,  half),
        Vec3::new( half, -half,  half),
        Vec3::new( half,  half,  half),
        Vec3::new(-half,  half,  half),
    ];
    // Yaw rotation + translate to world position.
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    for c in &mut corners {
        let rx = c.x * cos_y - c.z * sin_y;
        let rz = c.x * sin_y + c.z * cos_y;
        c.x = rx + centre.x;
        c.y += centre.y;
        c.z = rz + centre.z;
    }

    push_textured_quad(verts, tex_side, [1.0, 0.0, 0.0],
        corners[1], corners[5], corners[6], corners[2]);
    push_textured_quad(verts, tex_side, [-1.0, 0.0, 0.0],
        corners[4], corners[0], corners[3], corners[7]);
    push_textured_quad(verts, tex_top, [0.0, 1.0, 0.0],
        corners[3], corners[2], corners[6], corners[7]);
    push_textured_quad(verts, tex_bottom, [0.0, -1.0, 0.0],
        corners[4], corners[5], corners[1], corners[0]);
    push_textured_quad(verts, tex_side, [0.0, 0.0, 1.0],
        corners[5], corners[4], corners[7], corners[6]);
    push_textured_quad(verts, tex_side, [0.0, 0.0, -1.0],
        corners[0], corners[1], corners[2], corners[3]);

    // Campaign N — dropped loot dims with the world's light too.
    apply_light_channels(&mut verts[verts_start..], light);
}

/// Resolve a wire `ItemRef` to drop-cube textures. `None` = nothing sound to
/// render: unknown/hostile block ids (the registry's AIR fallback would draw
/// a ghost cube), unknown material discriminants, `Empty`. Tool tiers map the
/// same way as `held_item_model::held_item_mesh` so a dropped tool matches
/// its held look.
fn item_ref_textures(
    item: crate::protocol::ItemRef,
    registry: &crate::block::BlockRegistry,
) -> Option<(u32, u32, u32)> {
    use crate::crafting::ToolMaterial;
    use crate::protocol::ItemRef;
    match item {
        ItemRef::Block(id) if registry.is_known(id) => {
            let def = registry.get(id);
            Some((def.tex_top, def.tex_side, def.tex_bottom))
        }
        ItemRef::Material(id) => crate::item::MaterialId::try_from(id).ok().map(|m| {
            let tex = material_texture(m);
            (tex, tex, tex)
        }),
        ItemRef::Tool(tier) => {
            let mat = match tier {
                0 => ToolMaterial::Wood,
                1 => ToolMaterial::Stone,
                2 => ToolMaterial::Iron,
                3 => ToolMaterial::Diamond,
                4 => ToolMaterial::Satori,
                _ => return None,
            };
            let tex = tool_texture(mat);
            Some((tex, tex, tex))
        }
        _ => None,
    }
}

/// Build vertices for the server-broadcast remote item entities (death-drops
/// phase 2b) — the ghost twins of `build_item_entity_vertices`' local drops.
/// Same bob/spin/lighting; positions come from the server's per-tick
/// `EntityUpdate`s instead of local physics.
pub fn build_remote_item_vertices(
    items: &crate::remote_entities::RemoteItems,
    registry: &crate::block::BlockRegistry,
    time_ticks: u32,
    player_pos: Vec3,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    let anim_time = time_ticks as f32 / 20.0;
    for it in items.iter() {
        // Skip if too close to camera (avoid seeing the inside).
        if (it.pos - player_pos).length_squared() < 0.05 {
            continue;
        }
        // Death-drops phase 3 — a `full_item` payload (tool/armour) wins:
        // decode it back to a real `Item` and reuse the LOCAL drop's texture
        // resolution, so a server-broadcast iron pickaxe looks exactly like
        // one dropped by the client's own sim. Otherwise fall back to the
        // legacy `(kind, id)` pair.
        let tex = match crate::inventory::item_from_wire_full(&it.full) {
            Some(item) => item_textures(&item, registry),
            None => match item_ref_textures(it.item, registry) {
                Some(t) => t,
                None => continue,
            },
        };
        push_item_drop_cube(&mut verts, tex, it.pos, anim_time, light_at(it.pos));
    }
    verts
}

/// Map a tool-material tier to its dedicated item-drop texture layer.
pub fn tool_texture(material: crate::crafting::ToolMaterial) -> u32 {
    use crate::crafting::ToolMaterial as M;
    match material {
        M::Wood => TEX_ITEM_TOOL_WOOD,
        M::Stone => TEX_ITEM_TOOL_STONE,
        M::Iron => TEX_ITEM_TOOL_IRON,
        M::Diamond => TEX_ITEM_TOOL_DIAMOND,
        // Satori tools reuse the Satori item icon for now — Wave 25b polish
        // can add dedicated tool variants if Axolittle wants distinct icons.
        M::Satori => crate::block::TEX_ITEM_SATORI,
    }
}

/// Map a `MaterialId` to its dedicated item-drop texture layer.
pub fn material_texture(id: crate::item::MaterialId) -> u32 {
    use crate::item::MaterialId as M;
    match id {
        // Spec 40 — placeholder icon (reuse the stick sprite) until a bellows
        // texture is drawn; the bellows is an authoring tool, not a survival item.
        M::Bellows => TEX_ITEM_STICK,
        M::Stick => TEX_ITEM_STICK,
        M::Leather => TEX_ITEM_LEATHER,
        M::Feather => TEX_ITEM_FEATHER,
        M::Wool => TEX_ITEM_WOOL,
        M::Bone => TEX_ITEM_BONE,
        M::RawBeef => TEX_ITEM_RAW_BEEF,
        M::RawPorkchop => TEX_ITEM_RAW_PORKCHOP,
        M::RawChicken => TEX_ITEM_RAW_CHICKEN,
        M::RawMutton => TEX_ITEM_RAW_MUTTON,
        M::String => TEX_ITEM_STRING,
        // Bonemeal (Wave 14) shares the bone texture for now — same off-white
        // tone, identifiable on pickup via the hotbar name.
        M::Bonemeal => TEX_ITEM_BONE,
        // Wave 13 ore drops — use the ore-block textures themselves so the
        // dropped item looks like a chunk of ore. Wave 13b polish can add
        // dedicated nugget/lump textures.
        M::Coal => crate::block::TEX_COAL_ORE,
        M::RawIron => crate::block::TEX_IRON_ORE,
        M::Diamond => crate::block::TEX_DIAMOND_ORE,
        // Wave 6 smelting outputs.
        M::IronIngot => TEX_ITEM_IRON_INGOT,
        M::CookedBeef => TEX_ITEM_COOKED_BEEF,
        M::CookedPorkchop => TEX_ITEM_COOKED_PORKCHOP,
        M::CookedChicken => TEX_ITEM_COOKED_CHICKEN,
        M::CookedMutton => TEX_ITEM_COOKED_MUTTON,
        M::Arrow => TEX_ITEM_ARROW,
        // Satori item-drop — dedicated texture (Wave 25).
        M::Satori => crate::block::TEX_ITEM_SATORI,
        // Farming Tier 1 (Wave 26) — dedicated item-drop textures.
        M::WheatSeeds => crate::block::TEX_ITEM_WHEAT_SEEDS,
        M::Wheat => crate::block::TEX_ITEM_WHEAT,
        M::Bread => crate::block::TEX_ITEM_BREAD,
        M::Carrot => crate::block::TEX_ITEM_CARROT,
        M::Potato => crate::block::TEX_ITEM_POTATO,
        M::Flint => crate::block::TEX_ITEM_FLINT,
        // Wave 28 — corn + baked variants.
        M::CornSeeds => crate::block::TEX_ITEM_CORN_SEEDS,
        M::Corn => crate::block::TEX_ITEM_CORN,
        M::BakedCorn => crate::block::TEX_ITEM_BAKED_CORN,
        M::BakedPotato => crate::block::TEX_ITEM_BAKED_POTATO,
        M::BakedCarrot => crate::block::TEX_ITEM_BAKED_CARROT,
        // Wave 29 — log seasoning. All three log materials share the oak-log
        // side texture; the per-material `Item::color` tint provides the
        // green-cast / amber / charred visual differentiation. Dedicated
        // textures can land as a Phase 9 polish task if Axolittle wants
        // sharper visual distinction at item-drop range.
        M::GreenLog | M::SeasonedLog | M::KilnDriedLog => crate::block::TEX_OAK_LOG_SIDE,
        // Spec 23 — Papyrus Reed + Sheet (Foundation A of Build Schematics).
        M::PapyrusReed => crate::block::TEX_ITEM_PAPYRUS_REED,
        M::PapyrusSheet => crate::block::TEX_ITEM_PAPYRUS_SHEET,
        // Spec 28c Materials Expansion. Texture-pack work deferred; raw
        // copper reuses the iron-ore texture as a placeholder (tinted by
        // Item::color in the hotbar), ingots reuse iron-ingot, amethyst
        // reuses diamond. The per-material colour table differentiates
        // them visually until dedicated textures land.
        M::Copper => crate::block::TEX_IRON_ORE,
        M::Tin => crate::block::TEX_IRON_ORE,
        M::Sulphur => TEX_ITEM_GREY_POWDER,
        M::Amethyst => crate::block::TEX_DIAMOND_ORE,
        M::CopperIngot => TEX_ITEM_IRON_INGOT,
        M::TinIngot => TEX_ITEM_IRON_INGOT,
        M::BronzeIngot => TEX_ITEM_IRON_INGOT,
        M::Sugar => crate::block::TEX_ITEM_WHEAT,
        // T1.5 Processed Economy Base — texture-pack BRIDGE. All new
        // processed-food materials share neighbouring textures, tinted
        // by Item::color in the hotbar until dedicated textures land.
        // BRIDGE: T1.5 textures — replace per-material when the texture
        // pack is generated by texture_gen.rs Phase 9.
        M::Bucket => TEX_ITEM_IRON_INGOT,
        M::MilkBucket => TEX_ITEM_IRON_INGOT,
        // Reuse the bucket sprite; the per-material colour tint distinguishes them.
        M::WaterBucket => TEX_ITEM_IRON_INGOT,
        M::LavaBucket => TEX_ITEM_IRON_INGOT,
        M::Egg => crate::block::TEX_ITEM_BREAD,
        M::Flour => crate::block::TEX_ITEM_WHEAT,
        M::Dough => crate::block::TEX_ITEM_BREAD,
        M::Cream => crate::block::TEX_ITEM_BREAD,
        M::Butter => crate::block::TEX_ITEM_BREAD,
        M::Cheese => crate::block::TEX_ITEM_BREAD,
        M::SweetBread => crate::block::TEX_ITEM_BREAD,
        M::Cake => crate::block::TEX_ITEM_BREAD,
        M::PumpkinPie => crate::block::TEX_ITEM_BAKED_CARROT,
        M::BerryPie => crate::block::TEX_ITEM_BAKED_CARROT,
        M::Cookie => crate::block::TEX_ITEM_BREAD,
        M::Pancakes => crate::block::TEX_ITEM_BREAD,
        M::LoadedBakedPotato => crate::block::TEX_ITEM_BAKED_POTATO,
        M::Stew => crate::block::TEX_ITEM_BAKED_CARROT,
        M::BeetrootSoup => crate::block::TEX_ITEM_BAKED_CARROT,
        M::Bowl => crate::block::TEX_OAK_LOG_SIDE,
        M::PumpkinFood => crate::block::TEX_ITEM_CARROT,
        M::SugarBeet => crate::block::TEX_ITEM_CARROT,
        M::SugarBeetSeeds => crate::block::TEX_ITEM_WHEAT_SEEDS,
        M::Beetroot => crate::block::TEX_ITEM_CARROT,
        M::BeetrootSeeds => crate::block::TEX_ITEM_WHEAT_SEEDS,
        M::Berries => crate::block::TEX_ITEM_CARROT,
        // Spec 28b — saplings share the wheat-seeds icon, tinted by
        // Item::color per species. BRIDGE: replace when species-specific
        // sapling textures land alongside per-species tree-gen.
        M::OakSapling
        | M::BirchSapling
        | M::SpruceSapling
        | M::JungleSapling
        | M::AcaciaSapling
        | M::DarkOakSapling => crate::block::TEX_ITEM_WHEAT_SEEDS,
        // 28c Phase 5 — placeholder textures for the four mob-drop
        // materials. BRIDGE: replace when 28d Mobs lands the bee +
        // squid + glow-squid + future companions that drop these.
        M::Honeycomb | M::Honey => crate::block::TEX_ITEM_BREAD,
        M::InkSac => TEX_ITEM_GREY_POWDER,
        M::GlowBerry => crate::block::TEX_ITEM_CARROT,
        // 28d chunk 3 — Rabbit drops. RawRabbit reuses raw-chicken (kid
        // can tell from hotbar tint); CookedRabbit reuses cooked-chicken
        // for the same reason; hide reuses leather. BRIDGE: replace
        // when dedicated rabbit textures land in texture_gen.rs.
        M::RawRabbit => TEX_ITEM_RAW_CHICKEN,
        M::CookedRabbit => TEX_ITEM_COOKED_CHICKEN,
        M::RabbitHide => TEX_ITEM_LEATHER,
        // 28d chunk 5 — Bee drops. HoneyBottle reuses MilkBucket icon
        // tinted gold by Item::color; Stinger reuses the gunpowder
        // texture as a placeholder. BRIDGE: replace with dedicated
        // textures when texture_gen.rs grows the hive surface.
        M::HoneyBottle => crate::block::TEX_ITEM_BREAD,
        M::BeeStinger => TEX_ITEM_GREY_POWDER,
        // Spec 28d.nostrich — reuse existing item textures tinted
        // purple by Item::color. Dedicated procedural sprites land
        // in a follow-up texture-gen pass.
        M::NostrichFeather => TEX_ITEM_FEATHER,
        M::NostrichEgg => crate::block::TEX_ITEM_BREAD,
        M::RawNostrichMeat => TEX_ITEM_RAW_CHICKEN,
        M::RoyalPavlova => crate::block::TEX_ITEM_BREAD,
        M::NostrichOmelette => crate::block::TEX_ITEM_BREAD,
        M::NostrichCustard => crate::block::TEX_ITEM_BREAD,
        M::NostrichArrow => TEX_ITEM_ARROW,
        M::PurpleBanner => TEX_ITEM_WOOL,
        // HP-1 — Brigand Chieftain Trophy reuses the bone texture as a
        // placeholder, tinted maroon by Item::color. BRIDGE: replace
        // when texture_gen.rs lands a dedicated banner-trophy texture.
        M::BrigandChieftainTrophy => TEX_ITEM_BONE,
        // Salt — reuses the wool texture (white granular look) tinted
        // pink by Item::color. BRIDGE: dedicated salt-grain item sprite
        // would land in a follow-on polish round.
        M::Salt => TEX_ITEM_WOOL,
        // Spec 49 (Explosives) — saltpetre reuses the white granular wool sprite
        // (tinted pale by Item::color); black powder reuses the grey-powder sprite.
        // BRIDGE: dedicated saltpetre/black-powder item sprites post-playtest.
        M::Saltpetre => TEX_ITEM_WOOL,
        M::BlackPowder => TEX_ITEM_GREY_POWDER,
        // Spec 49 — Compost reuses the dirt face (earthy brown, tinted by color).
        M::Compost => crate::block::TEX_DIRT,
        // Cured raws reuse the raw-meat sprite of the same species,
        // tinted by Item::color. BRIDGE: dedicated cured-meat sprites
        // post-playtest.
        M::SaltCuredBeef => TEX_ITEM_RAW_BEEF,
        M::SaltCuredPorkchop => TEX_ITEM_RAW_PORKCHOP,
        M::SaltCuredMutton => TEX_ITEM_RAW_MUTTON,
        M::SaltCuredChicken => TEX_ITEM_RAW_CHICKEN,
        // Rabbit and Nostrich don't have dedicated raw textures; fall
        // back to a generic raw look via the species' base sprite.
        M::SaltCuredRabbit => TEX_ITEM_RAW_BEEF,
        M::SaltCuredNostrichMeat => TEX_ITEM_RAW_BEEF,
        // Seasoned cooked variants reuse the cooked sprite path.
        // BRIDGE: dedicated seasoned-cooked sprites post-playtest.
        M::SeasonedBread => TEX_ITEM_BONE, // placeholder
        M::SeasonedBakedPotato => TEX_ITEM_BONE, // placeholder
        M::SeasonedBakedCarrot => TEX_ITEM_BONE, // placeholder
        M::SeasonedBakedCorn => TEX_ITEM_BONE, // placeholder
        M::SeasonedCookedBeef => TEX_ITEM_RAW_BEEF, // tinted
        M::SeasonedCookedPorkchop => TEX_ITEM_RAW_PORKCHOP,
        M::SeasonedCookedMutton => TEX_ITEM_RAW_MUTTON,
        M::SeasonedCookedChicken => TEX_ITEM_RAW_CHICKEN,
        M::SeasonedCookedRabbit => TEX_ITEM_RAW_BEEF,
        // Rubber feature — generic bone-tinted-by-Item::color sprite.
        // BRIDGE: dedicated rubber-item sprites post-playtest.
        M::Rubber => TEX_ITEM_BONE,
        M::RubberSapling => TEX_ITEM_BONE,
        M::RubberBall => TEX_ITEM_BONE,
        M::CopperCable => TEX_ITEM_BONE,
        // Mob Bounty Board — placed-block item; uses the block face
        // tex directly so the dropped item looks like a board.
        M::BountyBoardItem => crate::block::TEX_BOUNTY_BOARD,
        // Tip Jar — same pattern; placed-block item uses block tex.
        M::TipJarItem => crate::block::TEX_TIP_JAR,
        // Repair Bench — placed-block item uses block tex.
        M::RepairBenchItem => crate::block::TEX_REPAIR_BENCH,
        // Plot Marker — placed-block item uses block tex.
        M::PlotMarkerItem => crate::block::TEX_PLOT_MARKER,
        // Market Bell — placed-block item uses block tex.
        M::MarketBellItem => crate::block::TEX_MARKET_BELL,
        // Auction Block — placed-block item uses block tex.
        M::AuctionBlockItem => crate::block::TEX_AUCTION_BLOCK,
        // Bazaar Block — placed-block item uses block tex.
        M::BazaarBlockItem => crate::block::TEX_BAZAAR_BLOCK,
        // Dyes (Spec 35) + fibre/cordage (Spec 36) — dedicated item icons.
        M::BlueDye => TEX_ITEM_BLUE_DYE,
        M::RedDye => TEX_ITEM_RED_DYE,
        M::YellowDye => TEX_ITEM_YELLOW_DYE,
        M::Cotton => TEX_ITEM_COTTON,
        M::HempFibre => TEX_ITEM_HEMP_FIBRE,
        M::Rope => TEX_ITEM_ROPE,
        // Magnesium (Spec 37).
        M::Magnesium => TEX_ITEM_MAGNESIUM,
        M::Fertiliser => TEX_ITEM_FERTILISER,
        M::Sparkler => TEX_ITEM_SPARKLER,
        M::Flare => TEX_ITEM_FLARE,
        M::MagnesiumFirestarter => TEX_ITEM_FIRESTARTER,
        // Dye Phase 2 (Spec 35).
        M::BlackDye => TEX_ITEM_BLACK_DYE,
        M::WhiteDye => TEX_ITEM_WHITE_DYE,
        M::OrangeDye => TEX_ITEM_ORANGE_DYE,
        M::GreenDye => TEX_ITEM_GREEN_DYE,
        M::PurpleDye => TEX_ITEM_PURPLE_DYE,
        M::PinkDye => TEX_ITEM_PINK_DYE,
        M::LimeDye => TEX_ITEM_LIME_DYE,
        M::LightBlueDye => TEX_ITEM_LIGHT_BLUE_DYE,
        M::GreyDye => TEX_ITEM_GREY_DYE,
        M::LightGreyDye => TEX_ITEM_LIGHT_GREY_DYE,
        // Fibre Phase 2 seeds (Spec 36).
        M::CottonSeeds => TEX_ITEM_COTTON_SEEDS,
        M::HempSeeds => TEX_ITEM_HEMP_SEEDS,
        // Spec 35 Phase 2 completion (2026-05-28) — 3-input mix dyes.
        M::BrownDye => TEX_ITEM_BROWN_DYE,
        M::CyanDye => TEX_ITEM_CYAN_DYE,
        M::MagentaDye => TEX_ITEM_MAGENTA_DYE,
        // Spec 36 Phase 2 (2026-05-28) — Rope-consumer + textiles.
        M::Lead => TEX_ITEM_LEAD,
        M::Cloth => TEX_ITEM_CLOTH,
        M::Canvas => TEX_ITEM_CANVAS,
        // Spec 35 farmable-flower follow-on (2026-05-28).
        M::CornflowerSeeds => TEX_ITEM_CORNFLOWER_SEEDS,
        M::FieldPoppySeeds => TEX_ITEM_FIELD_POPPY_SEEDS,
        M::ButtercupSeeds => TEX_ITEM_BUTTERCUP_SEEDS,
        // Craftable Armoured Carts (CA2) — placeholder icons reusing the hull
        // material's block texture (planks / iron / diamond), tinted per-tier
        // by Item::color so the dropped item reads as a cart of that armour
        // tier. BRIDGE: dedicated cart-item sprites land when texture_gen.rs
        // grows a cart icon (CA3/CA4 render the actual cart entity model).
        M::WoodCart => crate::block::TEX_OAK_PLANKS,
        M::IronCart => crate::block::TEX_IRON_BLOCK,
        M::DiamondCart => crate::block::TEX_DIAMOND_BLOCK,
        // P6 — fishing catch. BRIDGE: reuse the chicken sprites (Item::color
        // tints them blue-grey/golden) until dedicated fish icons are drawn.
        M::RawFish => TEX_ITEM_RAW_CHICKEN,
        // Aquatic wave — reuse closest sprites (bone-ivory tooth, grey-powder
        // ink) until dedicated icons are drawn.
        M::SharkTooth => TEX_ITEM_BONE,
        M::GlowInk => TEX_ITEM_GREY_POWDER,
        M::CookedFish => TEX_ITEM_COOKED_CHICKEN,
        // Pets wave Task 7 — reuse the bone sprite (the whistle is carved
        // from bone) until a dedicated icon is drawn.
        M::RecallWhistle => TEX_ITEM_BONE,
        // Pets wave Task 9 — reuse the bone sprite (biscuit-shaped, same
        // placeholder convention as RecallWhistle/SharkTooth) until a
        // dedicated icon is drawn.
        M::CatTreat => TEX_ITEM_BONE,
        // Pets wave Task 13 — Crab Claw + the Reach Claw built from it both
        // reuse the bone sprite (claw-shaped, same placeholder convention as
        // SharkTooth/RecallWhistle/CatTreat) until dedicated icons are drawn;
        // `Item::color` tints them apart (ivory claw vs. rust-red tool).
        M::CrabClaw => TEX_ITEM_BONE,
        M::ReachClaw => TEX_ITEM_BONE,
    }
}

/// Build vertices for in-flight projectile (arrow) entities. Each arrow is
/// rendered as a thin elongated cuboid stretched along its velocity direction
/// — quick-and-readable rather than animated. Wave 23.
pub fn build_projectile_vertices(
    ecs: &hecs::World,
    player_pos: Vec3,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    use crate::entity::ProjectileEntity;
    let mut verts = Vec::new();
    for (_id, (pos, vel, _proj)) in ecs
        .query::<(&crate::entity::Position, &crate::entity::Velocity, &ProjectileEntity)>()
        .iter()
    {
        push_arrow(&mut verts, pos.0, vel.0, player_pos, &light_at);
    }
    verts
}

/// Build vertices for the server-broadcast projectiles (MP-A3) — the very same
/// arrow `build_projectile_vertices` draws for the local sim's own, posed from
/// the server's per-tick positions and the heading between them.
pub fn build_remote_projectile_vertices(
    arrows: &crate::remote_entities::RemoteProjectiles,
    player_pos: Vec3,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    for a in arrows.iter() {
        push_arrow(&mut verts, a.pos, a.dir, player_pos, &light_at);
    }
    verts
}

/// One arrow: a thin cuboid at `pos` pointing along `heading` (any length —
/// only its direction is used). Skipped when it sits right at the camera
/// (avoid "inside the model").
fn push_arrow(
    verts: &mut Vec<Vertex>,
    pos: Vec3,
    heading: Vec3,
    player_pos: Vec3,
    light_at: &impl Fn(Vec3) -> (f32, f32),
) {
    if (pos - player_pos).length_squared() < 0.04 {
        return;
    }
    let verts_start = verts.len();
    // Yaw + pitch from the heading. Arrow shaft points along its travel.
    let v = heading;
    let yaw = (-v.x).atan2(-v.z);
    let horiz_len = (v.x * v.x + v.z * v.z).sqrt();
    let pitch = (-v.y).atan2(horiz_len.max(1e-4));

    // Arrow cuboid: long along Z (length 0.5), thin in X/Y (0.06).
    let half_x = 0.03;
    let half_y = 0.03;
    let half_z = 0.25;

    let mut corners = [
        Vec3::new(-half_x, -half_y, -half_z),
        Vec3::new( half_x, -half_y, -half_z),
        Vec3::new( half_x,  half_y, -half_z),
        Vec3::new(-half_x,  half_y, -half_z),
        Vec3::new(-half_x, -half_y,  half_z),
        Vec3::new( half_x, -half_y,  half_z),
        Vec3::new( half_x,  half_y,  half_z),
        Vec3::new(-half_x,  half_y,  half_z),
    ];
    // Pitch (around X axis).
    let cos_p = pitch.cos();
    let sin_p = pitch.sin();
    for c in &mut corners {
        let dy = c.y;
        let dz = c.z;
        c.y = dy * cos_p - dz * sin_p;
        c.z = dy * sin_p + dz * cos_p;
    }
    // Yaw (around Y axis) + translate to world.
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    for c in &mut corners {
        let rx = c.x * cos_y - c.z * sin_y;
        let rz = c.x * sin_y + c.z * cos_y;
        c.x = rx + pos.x;
        c.y += pos.y;
        c.z = rz + pos.z;
    }

    let tex = TEX_ITEM_ARROW;
    push_textured_quad(verts, tex, [1.0, 0.0, 0.0],
        corners[1], corners[5], corners[6], corners[2]);
    push_textured_quad(verts, tex, [-1.0, 0.0, 0.0],
        corners[4], corners[0], corners[3], corners[7]);
    push_textured_quad(verts, tex, [0.0, 1.0, 0.0],
        corners[3], corners[2], corners[6], corners[7]);
    push_textured_quad(verts, tex, [0.0, -1.0, 0.0],
        corners[4], corners[5], corners[1], corners[0]);
    push_textured_quad(verts, tex, [0.0, 0.0, 1.0],
        corners[5], corners[4], corners[7], corners[6]);
    push_textured_quad(verts, tex, [0.0, 0.0, -1.0],
        corners[0], corners[1], corners[2], corners[3]);

    // Campaign N — projectiles dim with the world's light.
    apply_light_channels(&mut verts[verts_start..], light_at(pos));
}

// ── Rail freight (Phase 1) — cart render ─────────────────────────────────────

/// A single yaw-rotated cuboid of the cart, expressed as an offset + half-size
/// from the cart's anchor (foot centre). Faces all share one texture layer for
/// the Phase 1 placeholder look.
struct CartBox {
    /// Centre offset from the cart anchor (before yaw), in blocks.
    centre: Vec3,
    /// Half-extents (x, y, z), in blocks.
    half: Vec3,
    /// Texture layer for every face.
    tex: u32,
}

/// The cart's box body + 4 wheels. Reuses existing block textures (oak planks
/// for the body, oak-log-top end-grain for the wheels) so this task adds no new
/// texture layers. Coordinates are relative to the cart anchor (the cell-centre
/// `Position`), Y measured up from the track-slab surface.
// BRIDGE: rebuilds the 5-box layout per frame (mirrors the other per-call
// entity vertex builders). Promote to a `LazyLock<Vec<CartBox>>` like
// `MODEL_CACHE` if cart counts ever make the per-frame alloc measurable.
fn cart_boxes() -> Vec<CartBox> {
    let body = crate::block::TEX_OAK_PLANKS;
    let wheel = crate::block::TEX_OAK_LOG_TOP;
    // Body spans roughly one block in X/Z, sits a little above the slab. Wheels
    // are small flat discs (cuboids) at the four lower corners.
    let wheel_y = 0.12;
    let wheel_r = 0.12;
    let wx = 0.34; // wheel inset from centre on X
    let wz = 0.30; // wheel inset from centre on Z
    vec![
        // Body tub.
        CartBox {
            centre: Vec3::new(0.0, 0.34, 0.0),
            half: Vec3::new(0.40, 0.22, 0.34),
            tex: body,
        },
        // Four wheels.
        CartBox { centre: Vec3::new(wx, wheel_y, wz), half: Vec3::new(0.06, wheel_r, wheel_r), tex: wheel },
        CartBox { centre: Vec3::new(-wx, wheel_y, wz), half: Vec3::new(0.06, wheel_r, wheel_r), tex: wheel },
        CartBox { centre: Vec3::new(wx, wheel_y, -wz), half: Vec3::new(0.06, wheel_r, wheel_r), tex: wheel },
        CartBox { centre: Vec3::new(-wx, wheel_y, -wz), half: Vec3::new(0.06, wheel_r, wheel_r), tex: wheel },
    ]
}

/// Emit one yaw-rotated cuboid box at `anchor` (cart foot centre) into `verts`.
fn push_cart_box(verts: &mut Vec<Vertex>, b: &CartBox, anchor: Vec3, yaw: f32) {
    let h = b.half;
    let c = b.centre;
    let mut corners = [
        Vec3::new(c.x - h.x, c.y - h.y, c.z - h.z), // 0: ---
        Vec3::new(c.x + h.x, c.y - h.y, c.z - h.z), // 1: +--
        Vec3::new(c.x + h.x, c.y + h.y, c.z - h.z), // 2: ++-
        Vec3::new(c.x - h.x, c.y + h.y, c.z - h.z), // 3: -+-
        Vec3::new(c.x - h.x, c.y - h.y, c.z + h.z), // 4: --+
        Vec3::new(c.x + h.x, c.y - h.y, c.z + h.z), // 5: +-+
        Vec3::new(c.x + h.x, c.y + h.y, c.z + h.z), // 6: +++
        Vec3::new(c.x - h.x, c.y + h.y, c.z + h.z), // 7: -++
    ];
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    for p in &mut corners {
        let rx = p.x * cos_y - p.z * sin_y;
        let rz = p.x * sin_y + p.z * cos_y;
        p.x = rx + anchor.x;
        p.y += anchor.y;
        p.z = rz + anchor.z;
    }
    let t = b.tex;
    push_textured_quad(verts, t, [1.0, 0.0, 0.0], corners[1], corners[5], corners[6], corners[2]);
    push_textured_quad(verts, t, [-1.0, 0.0, 0.0], corners[4], corners[0], corners[3], corners[7]);
    push_textured_quad(verts, t, [0.0, 1.0, 0.0], corners[3], corners[2], corners[6], corners[7]);
    push_textured_quad(verts, t, [0.0, -1.0, 0.0], corners[4], corners[5], corners[1], corners[0]);
    push_textured_quad(verts, t, [0.0, 0.0, 1.0], corners[5], corners[4], corners[7], corners[6]);
    push_textured_quad(verts, t, [0.0, 0.0, -1.0], corners[0], corners[1], corners[2], corners[3]);
}

/// Build vertices for every cart in the ECS (Rail freight Phase 1). Mirrors
/// `build_item_entity_vertices` / `build_projectile_vertices`: queries the
/// render-only `Position` + `CartData`, yaw-orients the body by `CartData.facing`,
/// and emits the body + 4 wheels. Block-textured, so it rides the same entity
/// pipeline as item drops.
pub fn build_cart_vertices(
    ecs: &hecs::World,
    player_pos: Vec3,
    light_at: impl Fn(Vec3) -> (f32, f32),
) -> Vec<Vertex> {
    let mut verts = Vec::new();
    let boxes = cart_boxes();
    for (_id, (pos, cart)) in ecs
        .query::<(&Position, &crate::cart::CartData)>()
        .iter()
    {
        // Skip if right on the camera (avoid being inside the model).
        if (pos.0 - player_pos).length_squared() < 0.02 {
            continue;
        }
        let verts_start = verts.len();
        for b in &boxes {
            push_cart_box(&mut verts, b, pos.0, cart.facing);
        }
        // Campaign N — carts dim with the world's light.
        apply_light_channels(&mut verts[verts_start..], light_at(pos.0));
    }
    verts
}

/// Peak arm-swing rotation (radians) and the swing-window duration in ticks.
pub const SWING_PEAK: f32 = 1.2; // radians
pub const SWING_TICKS: u32 = 6; // arm-swing duration in ticks
/// Mining/placing arm swing, t in [0,1] over the swing window. 0 at both ends, single peak.
pub fn swing_angle(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    SWING_PEAK * (1.0 - t) * (std::f32::consts::PI * t).sin()
}

/// Per-part rotation inputs for `build_part_vertices`. Keeps call sites readable
/// now that the head can pitch and one arm can override the walk cycle.
#[derive(Clone, Copy)]
pub struct PartPose {
    /// Existing per-part walk-cycle X rotation.
    pub walk_swing: f32,
    /// X rotation applied only when `part.pitch_tracks_look`.
    pub head_pitch: f32,
    /// When `Some`, REPLACES `walk_swing` for this part (the swing arm).
    pub arm_override: Option<f32>,
    /// #19 Phase C (rung 2, squash & stretch) — per-axis non-uniform scale of the
    /// part's corner offsets **from its pivot**, applied BEFORE the X rotation.
    /// `[1,1,1]` (the default) is a no-op and is skipped entirely, so every
    /// existing avatar/mob/cart emit stays byte-identical to pre-Phase-C output.
    pub scale: [f32; 3],
}

impl Default for PartPose {
    fn default() -> Self {
        Self { walk_swing: 0.0, head_pitch: 0.0, arm_override: None, scale: [1.0; 3] }
    }
}

/// The effective X rotation for a part: the swing-arm override wins over the walk
/// cycle, and a look-tracking part additionally pitches. Factored out of
/// [`build_part_vertices`] so the #19 rigged **shell** path (which emits a baked
/// micro-model's vertices instead of 8 cuboid corners) poses through the exact
/// same rule.
pub(crate) fn pose_x_rot(pose: PartPose, pitch_tracks_look: bool) -> f32 {
    let mut x_rot = pose.arm_override.unwrap_or(pose.walk_swing);
    if pitch_tracks_look {
        x_rot += pose.head_pitch;
    }
    x_rot
}

/// Pose a set of model-space points about `pivot` and place them in the world:
/// squash/stretch the offsets-from-pivot (`scale`, #19 Phase C — skipped when
/// identity), rotate about the pivot on X by `x_rot` (the joint rotation, rung
/// 1), then yaw about Y and translate to `entity_pos`.
///
/// This is the canonical part transform, shared by the cuboid path
/// ([`build_part_vertices`]) and the rigged micro-model **shell** path
/// ([`build_rigged_vertices`]) so a shell part and a cuboid part move identically.
pub(crate) fn pose_points_about_pivot(
    points: &mut [Vec3],
    pivot: Vec3,
    scale: [f32; 3],
    x_rot: f32,
    yaw: f32,
    entity_pos: Vec3,
) {
    // Rung 2 — squash & stretch, about the pivot, BEFORE the rotation.
    if scale != [1.0, 1.0, 1.0] {
        for p in points.iter_mut() {
            p.x = pivot.x + (p.x - pivot.x) * scale[0];
            p.y = pivot.y + (p.y - pivot.y) * scale[1];
            p.z = pivot.z + (p.z - pivot.z) * scale[2];
        }
    }

    // Rung 1 — swing/pitch animation (rotate around the pivot on the X axis).
    if x_rot.abs() > 0.001 {
        let cos_a = x_rot.cos();
        let sin_a = x_rot.sin();
        for p in points.iter_mut() {
            let dy = p.y - pivot.y;
            let dz = p.z - pivot.z;
            p.y = pivot.y + dy * cos_a - dz * sin_a;
            p.z = pivot.z + dy * sin_a + dz * cos_a;
        }
    }

    // Yaw rotation (around Y) + translate to the entity's world position.
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    for p in points.iter_mut() {
        let rx = p.x * cos_y - p.z * sin_y;
        let rz = p.x * sin_y + p.z * cos_y;
        p.x = rx + entity_pos.x;
        p.y += entity_pos.y;
        p.z = rz + entity_pos.z;
    }
}

fn build_part_vertices(
    verts: &mut Vec<Vertex>,
    part: &ModelPart,
    entity_pos: Vec3,
    yaw: f32,
    pose: PartPose,
    // Spec 40 (The Workshop): when `Some`, REPLACES `part.tex_faces` with an
    // authored reskin's resolved layers for this part (per-face, mixed with
    // defaults by the override registry). `None` ⇒ stock textures (unchanged).
    tex_override: Option<&[u32; 6]>,
    // Spec 40 (The Workshop) — uniform scale about the entity origin, for the
    // inflated mannequin/working-copy render. `1.0` = normal size.
    scale: f32,
) {
    let tex_faces = tex_override.unwrap_or(&part.tex_faces);
    let half = part.size * 0.5 * scale;
    // Part origin is centre-bottom of the part (scaled about the entity origin).
    let centre = part.origin * scale;

    // 8 corners of the cuboid (local to entity, before rotation)
    let mut corners = [
        Vec3::new(centre.x - half.x, centre.y - half.y, centre.z - half.z), // 0: ---
        Vec3::new(centre.x + half.x, centre.y - half.y, centre.z - half.z), // 1: +--
        Vec3::new(centre.x + half.x, centre.y + half.y, centre.z - half.z), // 2: ++-
        Vec3::new(centre.x - half.x, centre.y + half.y, centre.z - half.z), // 3: -+-
        Vec3::new(centre.x - half.x, centre.y - half.y, centre.z + half.z), // 4: --+
        Vec3::new(centre.x + half.x, centre.y - half.y, centre.z + half.z), // 5: +-+
        Vec3::new(centre.x + half.x, centre.y + half.y, centre.z + half.z), // 6: +++
        Vec3::new(centre.x - half.x, centre.y + half.y, centre.z + half.z), // 7: -++
    ];

    // Effective X rotation about the part pivot. The swing arm overrides the
    // walk cycle; the head additionally pitches with the look direction. Then
    // squash/stretch (Phase C, no-op by default) → X rotation about the pivot →
    // yaw + translate — the shared transform the rigged shell path also uses.
    let x_rot = pose_x_rot(pose, part.pitch_tracks_look);
    pose_points_about_pivot(&mut corners, part.pivot * scale, pose.scale, x_rot, yaw, entity_pos);

    // 6 faces with normals and texture layers
    // tex_faces: [+x, -x, +y, -y, +z, -z]

    // +X face (right): corners 1,5,6,2
    push_textured_quad(verts, tex_faces[0], [1.0, 0.0, 0.0],
        corners[1], corners[5], corners[6], corners[2]);
    // -X face (left): corners 4,0,3,7
    push_textured_quad(verts, tex_faces[1], [-1.0, 0.0, 0.0],
        corners[4], corners[0], corners[3], corners[7]);
    // +Y face (top): corners 3,2,6,7
    push_textured_quad(verts, tex_faces[2], [0.0, 1.0, 0.0],
        corners[3], corners[2], corners[6], corners[7]);
    // -Y face (bottom): corners 4,5,1,0
    push_textured_quad(verts, tex_faces[3], [0.0, -1.0, 0.0],
        corners[4], corners[5], corners[1], corners[0]);
    // +Z face (back): corners 5,4,7,6
    push_textured_quad(verts, tex_faces[4], [0.0, 0.0, 1.0],
        corners[5], corners[4], corners[7], corners[6]);
    // -Z face (front): corners 0,1,2,3
    push_textured_quad(verts, tex_faces[5], [0.0, 0.0, -1.0],
        corners[0], corners[1], corners[2], corners[3]);
}

/// #19 Rig Studio — emit the vertices for an authored rig at `pos`, posed by its
/// standard skeleton's inherited animation `clip` at `anim_time`.
///
/// Each rigged part renders one of two ways, decided per assigned block:
///   * the block has a **registered micro-model** (#18, `micro_registry`) ⇒ the
///     part renders as that block's **baked shell**, scaled to fit the skeleton
///     part's `size` box (aspect preserved) and centred on the part origin — the
///     spec's "attach the baked shell, not a cuboid" step;
///   * otherwise ⇒ the original cuboid of the assigned block (unchanged, so every
///     rig authored before shell attach looks exactly as it did).
///
/// The bake itself is **not** repeated per frame: `MicroModelRegistry::register`
/// bakes once per `BlockId` and this path only reads the cached `ChunkMesh`. Both
/// paths pose through the same pivot/scale/rotation transform
/// ([`pose_points_about_pivot`]), so a shell part and a cuboid part swing alike.
///
/// `micro` is `None` on call sites with no registry to hand (tests, tools) — that
/// simply means "cuboids only".
pub fn build_rigged_vertices(
    verts: &mut Vec<Vertex>,
    rig: &crate::skeleton::RiggedModel,
    registry: &crate::block::BlockRegistry,
    micro: Option<&crate::micro_model_registry::MicroModelRegistry>,
    pos: Vec3,
    yaw: f32,
    clip: crate::anim_set::AnimClip,
    anim_time: f32,
    // Campaign N — pre-sampled `(block, sky)` channels at the rig's position
    // (rigs are stationary, so the call site samples once per rig).
    light: (f32, f32),
) {
    let verts_start = verts.len();
    let skeleton = rig.skeleton;
    let parts = skeleton.parts();
    let swing_part = skeleton.swing_part();
    for rp in &rig.parts {
        let Some(sp) = parts.get(rp.skeleton_part) else {
            continue;
        };
        let block = rp.micro_model_block;
        if block == crate::block::AIR {
            continue;
        }
        let pivot = rig.pivot_of(rp);
        let is_swing = swing_part == Some(sp.name);
        let pose = crate::anim_set::eval_anim_set(clip, anim_time, sp, is_swing);

        // Shell attach: a block with a registered micro-model draws its baked,
        // interior-culled shell instead of a flat-textured box.
        if let Some(baked) = micro.and_then(|m| m.get(block)) {
            push_rigged_shell(verts, &baked.mesh, sp, pivot, pose, pos, yaw);
            continue;
        }

        // Texture the cuboid from the assigned block's faces.
        let side = registry.tex_side(block);
        let tex = [
            side,
            side,
            registry.tex_top(block),
            registry.tex_bottom(block),
            side,
            side,
        ];
        let mut mp = sp.to_model_part();
        mp.tex_faces = tex;
        mp.pivot = pivot;
        build_part_vertices(verts, &mp, pos, yaw, pose, None, 1.0);
    }
    apply_light_channels(&mut verts[verts_start..], light);
}

/// Fit a baked micro-model shell into a skeleton part's box and emit it posed.
///
/// The shell's baked vertices live in the host block's unit cube; this scales
/// them **uniformly** (so the author's proportions survive — the largest axis
/// that still fits wins), recentres the shell's bounding box on the skeleton
/// part's `origin`, and then runs the shared pivot transform. The baked mesh is
/// indexed; the entity pipeline is a plain triangle list, so indices are expanded
/// (emitted vertex count == `mesh.indices.len()`).
fn push_rigged_shell(
    verts: &mut Vec<Vertex>,
    mesh: &crate::mesh::ChunkMesh,
    sp: &crate::skeleton::SkeletonPart,
    pivot: Vec3,
    pose: PartPose,
    entity_pos: Vec3,
    yaw: f32,
) {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        return;
    }
    // Bounding box of the baked shell (cheap: baked shells are tens-to-hundreds
    // of vertices, and we walk them again to transform anyway).
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for v in &mesh.vertices {
        let p = Vec3::from(v.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let extent = hi - lo;
    // Uniform fit: the tightest axis ratio, so the shell fills the part box
    // without ever overflowing it and without distorting the author's shape.
    let mut fit = f32::INFINITY;
    for a in 0..3 {
        if extent[a] > 1e-6 {
            fit = fit.min(sp.size[a] / extent[a]);
        }
    }
    if !fit.is_finite() || fit <= 0.0 {
        return;
    }
    let shell_centre = (lo + hi) * 0.5;

    let x_rot = pose_x_rot(pose, sp.pitch_tracks_look);

    // Model-space positions: recentre on the part origin, scaled to fit.
    let mut points: Vec<Vec3> = mesh
        .vertices
        .iter()
        .map(|v| sp.origin + (Vec3::from(v.position) - shell_centre) * fit)
        .collect();
    pose_points_about_pivot(&mut points, pivot, pose.scale, x_rot, yaw, entity_pos);

    // Normals follow the same rotation (no translation, no scale) so the shell
    // shades correctly as it swings.
    let (cos_a, sin_a) = (x_rot.cos(), x_rot.sin());
    let (cos_y, sin_y) = (yaw.cos(), yaw.sin());
    let rotate_normal = |n: [f32; 3]| -> [f32; 3] {
        let (mut y, mut z) = (n[1], n[2]);
        if x_rot.abs() > 0.001 {
            let (py, pz) = (y, z);
            y = py * cos_a - pz * sin_a;
            z = py * sin_a + pz * cos_a;
        }
        [n[0] * cos_y - z * sin_y, y, n[0] * sin_y + z * cos_y]
    };

    verts.reserve(mesh.indices.len());
    for &i in &mesh.indices {
        // Defensive: a hand-authored / hostile asset could carry a stale index.
        let Some(src) = mesh.vertices.get(i as usize) else {
            continue;
        };
        let mut v = *src;
        v.position = points[i as usize].to_array();
        v.normal = rotate_normal(src.normal);
        verts.push(v);
    }
}

/// Emit a SINGLE model part's vertices in LOCAL/model space, recentred on the
/// part's own origin (so the part centre sits at `(0,0,0)`), with no entity
/// yaw and no entity translation — for the first-person viewmodel
/// (`viewmodel.rs`), which then transforms the whole part straight into view
/// space. The arm-swing (`pose.arm_override`, falling back to `walk_swing`,
/// plus head pitch if the part tracks look) is applied about the part's pivot
/// expressed relative to that recentred origin — matching the swing axis used
/// by `build_part_vertices` so the in-hand arm bends the same way the avatar's
/// does. Faces use the same winding/normals as `build_part_vertices`, but UVs
/// come from the 64x64 skin atlas via [`push_skin_quad`] instead of
/// `part.tex_faces[k]`: the player's own hand shows their uploaded skin
/// (Phase 2). This is the LOCAL twin of [`build_skin_part_vertices`] (the
/// world-space 3rd-person builder). Only
/// two things differ: (a) `half` is grown by `inflate` on every axis (so the
/// sleeve overlay box sits just outside the base and the alpha-cutout shader can
/// show the arm through transparent overlay pixels without z-fighting), and
/// (b) each face's UV comes from `uv_rects[k]` (box-unwrap atlas rect) via
/// [`push_skin_quad`] instead of `part.tex_faces[k]` via the block-array path.
/// `uv_rects` is in the same face order as `tex_faces`: `[+x,-x,+y,-y,+z,-z]`.
pub(crate) fn build_skin_part_local(
    part: &ModelPart,
    uv_rects: &[crate::skin_uv::UvRect; 6],
    inflate: f32,
    pose: PartPose,
    skin_layer: u32,
) -> Vec<Vertex> {
    let half = part.size * 0.5 + Vec3::splat(inflate);
    // Corners centred on the origin (the part's own centre).
    let mut corners = [
        Vec3::new(-half.x, -half.y, -half.z), // 0: ---
        Vec3::new(half.x, -half.y, -half.z),  // 1: +--
        Vec3::new(half.x, half.y, -half.z),   // 2: ++-
        Vec3::new(-half.x, half.y, -half.z),  // 3: -+-
        Vec3::new(-half.x, -half.y, half.z),  // 4: --+
        Vec3::new(half.x, -half.y, half.z),   // 5: +-+
        Vec3::new(half.x, half.y, half.z),    // 6: +++
        Vec3::new(-half.x, half.y, half.z),   // 7: -++
    ];

    let mut x_rot = pose.arm_override.unwrap_or(pose.walk_swing);
    if part.pitch_tracks_look {
        x_rot += pose.head_pitch;
    }
    if x_rot.abs() > 0.001 {
        // Pivot relative to the recentred origin (origin is the part centre).
        let pivot = part.pivot - part.origin;
        let cos_a = x_rot.cos();
        let sin_a = x_rot.sin();
        for c in &mut corners {
            let dy = c.y - pivot.y;
            let dz = c.z - pivot.z;
            c.y = pivot.y + dy * cos_a - dz * sin_a;
            c.z = pivot.z + dy * sin_a + dz * cos_a;
        }
    }

    let mut verts = Vec::with_capacity(36);
    // +X (right): 1,5,6,2
    push_skin_quad(&mut verts, [1.0, 0.0, 0.0],
        corners[1], corners[5], corners[6], corners[2], uv_rects[0], skin_layer);
    // -X (left): 4,0,3,7
    push_skin_quad(&mut verts, [-1.0, 0.0, 0.0],
        corners[4], corners[0], corners[3], corners[7], uv_rects[1], skin_layer);
    // +Y (top): 3,2,6,7
    push_skin_quad(&mut verts, [0.0, 1.0, 0.0],
        corners[3], corners[2], corners[6], corners[7], uv_rects[2], skin_layer);
    // -Y (bottom): 4,5,1,0
    push_skin_quad(&mut verts, [0.0, -1.0, 0.0],
        corners[4], corners[5], corners[1], corners[0], uv_rects[3], skin_layer);
    // +Z (back): 5,4,7,6
    push_skin_quad(&mut verts, [0.0, 0.0, 1.0],
        corners[5], corners[4], corners[7], corners[6], uv_rects[4], skin_layer);
    // -Z (front): 0,1,2,3
    push_skin_quad(&mut verts, [0.0, 0.0, -1.0],
        corners[0], corners[1], corners[2], corners[3], uv_rects[5], skin_layer);
    verts
}

pub(crate) fn push_textured_quad(
    verts: &mut Vec<Vertex>,
    tex_layer: u32,
    normal: [f32; 3],
    a: Vec3, b: Vec3, c: Vec3, d: Vec3,
) {
    let a = a.to_array();
    let b = b.to_array();
    let c = c.to_array();
    let d = d.to_array();

    // Triangle 1: a, b, c
    verts.push(Vertex { position: a, normal, tex_layer, uv: [0.0, 1.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
    verts.push(Vertex { position: b, normal, tex_layer, uv: [1.0, 1.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
    verts.push(Vertex { position: c, normal, tex_layer, uv: [1.0, 0.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
    // Triangle 2: a, c, d
    verts.push(Vertex { position: a, normal, tex_layer, uv: [0.0, 1.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
    verts.push(Vertex { position: c, normal, tex_layer, uv: [1.0, 0.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
    verts.push(Vertex { position: d, normal, tex_layer, uv: [0.0, 0.0], light: Vertex::FULL_BRIGHT, sky_light: 0.0 });
}

/// Like [`push_textured_quad`] but maps the face to a sub-rectangle of the
/// 64x64 avatar skin atlas. `uv` = `[u0,v0,u1,v1]` in 0..1. `layer` is the
/// player's skin-array layer, carried in `Vertex.tex_layer` for `fs_avatar` to
/// sample (the avatar skin texture is a `texture_2d_array`).
///
/// Winding and vertex ORDER match `push_textured_quad` exactly, but the u ends
/// are the other way round: a = bottom-RIGHT, b = bottom-left, c = top-left,
/// d = top-right. That is not a quirk, it is the Minecraft box unwrap. The
/// classic layout is an unrolled cube — the head band runs
/// `right | front | left | back` across atlas x=0..32 — so neighbouring tiles
/// share real box edges, and walking forward around the model must walk forward
/// along the atlas. With this callers' corner order (a→b spanning -Z→+Z on the
/// +X face, and -X→+X on the -Z face) that requires u to run b→a, not a→b.
///
/// Ship the naive `push_textured_quad` mapping instead and every face renders
/// mirrored about its own vertical axis: an imported Minecraft skin reads
/// back-to-front on the sides of the head and text on the face comes out
/// reversed. `skin_hit::frac_uv` is the inverse of this and must be kept in
/// step, or the Workshop brush paints somewhere other than the crosshair.
/// Pinned by `skin_faces_follow_the_minecraft_box_unwrap`.
// Reached on ALL targets: the 1st-person viewmodel arm (`build_skin_part_local`)
// uses this on native + wasm32, and the 3rd-person avatar (`build_skin_part_vertices`)
// uses it on native. So it is no longer dead on wasm32.
pub(crate) fn push_skin_quad(
    verts: &mut Vec<Vertex>,
    normal: [f32; 3],
    a: Vec3, b: Vec3, c: Vec3, d: Vec3,
    uv: [f32; 4],
    layer: u32,
) {
    let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
    let a = a.to_array();
    let b = b.to_array();
    let c = c.to_array();
    let d = d.to_array();
    let fb = Vertex::FULL_BRIGHT;
    // Triangle 1: a, b, c
    verts.push(Vertex { position: a, normal, tex_layer: layer, uv: [u1, v1], light: fb, sky_light: 1.0 });
    verts.push(Vertex { position: b, normal, tex_layer: layer, uv: [u0, v1], light: fb, sky_light: 1.0 });
    verts.push(Vertex { position: c, normal, tex_layer: layer, uv: [u0, v0], light: fb, sky_light: 1.0 });
    // Triangle 2: a, c, d
    verts.push(Vertex { position: a, normal, tex_layer: layer, uv: [u1, v1], light: fb, sky_light: 1.0 });
    verts.push(Vertex { position: c, normal, tex_layer: layer, uv: [u0, v0], light: fb, sky_light: 1.0 });
    verts.push(Vertex { position: d, normal, tex_layer: layer, uv: [u1, v0], light: fb, sky_light: 1.0 });
}

/// `build_part_vertices`' twin for the 64x64 skin path. Identical transform
/// (pivot X-rotation → yaw → translate to `entity_pos`) and identical 6-face
/// corner orderings + normals; the only differences are (a) `half` is grown by
/// `inflate` on all axes (so the overlay box sits just outside the base) and
/// (b) each face's UV comes from `uv_rects[k]` (box-unwrap atlas rect) via
/// [`push_skin_quad`] instead of `part.tex_faces[k]` via the block-array path.
/// `uv_rects` is in the same face order as `tex_faces`: `[+x,-x,+y,-y,+z,-z]`.
// Only called from `build_player_avatar_vertices` (the 3rd-person avatar), which
// renders remote players — native-only, since the WASM client has no remote
// players yet. Dead on wasm32. (The 1st-person viewmodel uses the LOCAL twin
// `build_skin_part_local`, which IS reached on wasm32.)
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn build_skin_part_vertices(
    verts: &mut Vec<Vertex>,
    part: &ModelPart,
    uv_rects: &[crate::skin_uv::UvRect; 6],
    inflate: f32,
    entity_pos: Vec3,
    yaw: f32,
    pose: PartPose,
    skin_layer: u32,
    fade: f32,
) {
    // Phase 3 — record where this part's verts begin so the fade alpha can be
    // stamped into their `light` channel after they're pushed (full-bright
    // otherwise). The avatar pipeline reads `light` as a screen-door dither alpha.
    let fade_start = verts.len();
    let half = part.size * 0.5 + Vec3::splat(inflate);
    // Part origin is centre-bottom of the part
    let centre = part.origin;

    // 8 corners of the cuboid (local to entity, before rotation)
    let mut corners = [
        Vec3::new(centre.x - half.x, centre.y - half.y, centre.z - half.z), // 0: ---
        Vec3::new(centre.x + half.x, centre.y - half.y, centre.z - half.z), // 1: +--
        Vec3::new(centre.x + half.x, centre.y + half.y, centre.z - half.z), // 2: ++-
        Vec3::new(centre.x - half.x, centre.y + half.y, centre.z - half.z), // 3: -+-
        Vec3::new(centre.x - half.x, centre.y - half.y, centre.z + half.z), // 4: --+
        Vec3::new(centre.x + half.x, centre.y - half.y, centre.z + half.z), // 5: +-+
        Vec3::new(centre.x + half.x, centre.y + half.y, centre.z + half.z), // 6: +++
        Vec3::new(centre.x - half.x, centre.y + half.y, centre.z + half.z), // 7: -++
    ];

    // Effective X rotation about the part pivot. The swing arm overrides the
    // walk cycle; the head additionally pitches with the look direction.
    let mut x_rot = pose.arm_override.unwrap_or(pose.walk_swing);
    if part.pitch_tracks_look {
        x_rot += pose.head_pitch;
    }

    // Apply swing/pitch animation (rotate around pivot on X axis)
    if x_rot.abs() > 0.001 {
        let pivot = part.pivot;
        let cos_a = x_rot.cos();
        let sin_a = x_rot.sin();
        for c in &mut corners {
            let dy = c.y - pivot.y;
            let dz = c.z - pivot.z;
            c.y = pivot.y + dy * cos_a - dz * sin_a;
            c.z = pivot.z + dy * sin_a + dz * cos_a;
        }
    }

    // Apply yaw rotation (around Y axis) and translate to world position
    let cos_y = yaw.cos();
    let sin_y = yaw.sin();
    for c in &mut corners {
        let rx = c.x * cos_y - c.z * sin_y;
        let rz = c.x * sin_y + c.z * cos_y;
        c.x = rx + entity_pos.x;
        c.y += entity_pos.y;
        c.z = rz + entity_pos.z;
    }

    // 6 faces with normals and per-face skin UV rects.
    // uv_rects: [+x, -x, +y, -y, +z, -z]

    // +X face (right): corners 1,5,6,2
    push_skin_quad(verts, [1.0, 0.0, 0.0],
        corners[1], corners[5], corners[6], corners[2], uv_rects[0], skin_layer);
    // -X face (left): corners 4,0,3,7
    push_skin_quad(verts, [-1.0, 0.0, 0.0],
        corners[4], corners[0], corners[3], corners[7], uv_rects[1], skin_layer);
    // +Y face (top): corners 3,2,6,7
    push_skin_quad(verts, [0.0, 1.0, 0.0],
        corners[3], corners[2], corners[6], corners[7], uv_rects[2], skin_layer);
    // -Y face (bottom): corners 4,5,1,0
    push_skin_quad(verts, [0.0, -1.0, 0.0],
        corners[4], corners[5], corners[1], corners[0], uv_rects[3], skin_layer);
    // +Z face (back): corners 5,4,7,6
    push_skin_quad(verts, [0.0, 0.0, 1.0],
        corners[5], corners[4], corners[7], corners[6], uv_rects[4], skin_layer);
    // -Z face (front): corners 0,1,2,3
    push_skin_quad(verts, [0.0, 0.0, -1.0],
        corners[0], corners[1], corners[2], corners[3], uv_rects[5], skin_layer);

    // Phase 3 — stamp the fade alpha into this part's verts (full-bright when
    // fade==1.0, so the common opaque path is untouched). `fs_avatar` reads
    // `light` as the screen-door dither alpha; render-only, never affects aim.
    if fade < 1.0 {
        for v in verts[fade_start..].iter_mut() {
            v.light = fade;
        }
    }
}

// --- Spec 19 villager-family models ---

/// Villager — humanoid like a zombie but slightly taller (1.85 vs ~1.95).
/// Uses villager textures + a robe-style torso.
fn villager_model() -> Vec<ModelPart> {
    vec![
        // Torso (robe).
        ModelPart {
            origin: Vec3::new(0.0, 1.05, 0.0),
            size: Vec3::new(0.55, 0.85, 0.3),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        // Head.
        ModelPart {
            origin: Vec3::new(0.0, 1.65, 0.0),
            size: Vec3::new(0.55, 0.55, 0.55),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [
                TEX_VILLAGER_HEAD_SIDE, TEX_VILLAGER_HEAD_SIDE,
                TEX_VILLAGER_HEAD_SIDE, TEX_VILLAGER_HEAD_SIDE,
                TEX_VILLAGER_HEAD_FRONT, TEX_VILLAGER_HEAD_SIDE,
            ],
            pitch_tracks_look: false,
        },
        // Arms — robe-coloured.
        ModelPart {
            origin: Vec3::new(-0.4, 1.05, 0.0),
            size: Vec3::new(0.22, 0.8, 0.22),
            pivot: Vec3::new(-0.4, 1.4, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.4, 1.05, 0.0),
            size: Vec3::new(0.22, 0.8, 0.22),
            pivot: Vec3::new(0.4, 1.4, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        // Legs.
        ModelPart {
            origin: Vec3::new(-0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(-0.13, 0.7, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_VILLAGER_LEG; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(0.13, 0.7, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

/// Peddler — same skeleton as Villager with purple hood/robe.
fn peddler_model() -> Vec<ModelPart> {
    vec![
        ModelPart {
            origin: Vec3::new(0.0, 1.05, 0.0),
            size: Vec3::new(0.55, 0.85, 0.3),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_WANDERING_BODY; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.0, 1.65, 0.0),
            size: Vec3::new(0.55, 0.55, 0.55),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [
                TEX_WANDERING_HEAD_SIDE, TEX_WANDERING_HEAD_SIDE,
                TEX_WANDERING_HEAD_SIDE, TEX_WANDERING_HEAD_SIDE,
                TEX_WANDERING_HEAD_FRONT, TEX_WANDERING_HEAD_SIDE,
            ],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(-0.4, 1.05, 0.0),
            size: Vec3::new(0.22, 0.8, 0.22),
            pivot: Vec3::new(-0.4, 1.4, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_WANDERING_BODY; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.4, 1.05, 0.0),
            size: Vec3::new(0.22, 0.8, 0.22),
            pivot: Vec3::new(0.4, 1.4, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_WANDERING_BODY; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(-0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(-0.13, 0.7, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_WANDERING_LEG; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(0.13, 0.7, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_WANDERING_LEG; 6],
            pitch_tracks_look: false,
        },
    ]
}

/// Satoshi — the cloaked founder-sage. A villager skeleton in a brown robe with
/// a **hooded head** (the hood is textural, like the Peddler), a **cloak
/// back-drape**, and a small **glowing amulet** at the throat (the emissive part
/// — see [`satoshi_emissive_part`]). Warm, never scary: the face stays visible
/// under the hood. Reuses the brown villager robe/leg textures; only the hooded
/// head + glow trim are bespoke (`TEX_SATOSHI_*`).
fn satoshi_model() -> Vec<ModelPart> {
    vec![
        // 0: Torso — brown robe.
        ModelPart {
            origin: Vec3::new(0.0, 1.05, 0.0),
            size: Vec3::new(0.58, 0.9, 0.32),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        // 1: Hooded head — face on +Z (matches the villager's HEAD_FRONT face).
        ModelPart {
            origin: Vec3::new(0.0, 1.66, 0.0),
            size: Vec3::new(0.58, 0.58, 0.58),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [
                TEX_SATOSHI_HEAD_SIDE, TEX_SATOSHI_HEAD_SIDE,
                TEX_SATOSHI_HEAD_SIDE, TEX_SATOSHI_HEAD_SIDE,
                TEX_SATOSHI_HEAD_FRONT, TEX_SATOSHI_HEAD_SIDE,
            ],
            pitch_tracks_look: false,
        },
        // 2,3: Arms — robe-coloured, draped.
        ModelPart {
            origin: Vec3::new(-0.42, 1.05, 0.0),
            size: Vec3::new(0.24, 0.82, 0.24),
            pivot: Vec3::new(-0.42, 1.42, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.42, 1.05, 0.0),
            size: Vec3::new(0.24, 0.82, 0.24),
            pivot: Vec3::new(0.42, 1.42, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        // 4,5: Legs — mostly hidden under the robe.
        ModelPart {
            origin: Vec3::new(-0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(-0.13, 0.7, 0.0),
            animated: true,
            phase: 0.5,
            tex_faces: [TEX_VILLAGER_LEG; 6],
            pitch_tracks_look: false,
        },
        ModelPart {
            origin: Vec3::new(0.13, 0.35, 0.0),
            size: Vec3::new(0.22, 0.7, 0.22),
            pivot: Vec3::new(0.13, 0.7, 0.0),
            animated: true,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_LEG; 6],
            pitch_tracks_look: false,
        },
        // 6: Cloak back-drape — a thin wide panel behind the body (-Z is the
        //    back; the face is +Z). Gives the cloak silhouette from behind/side.
        ModelPart {
            origin: Vec3::new(0.0, 0.95, -0.20),
            size: Vec3::new(0.62, 1.15, 0.08),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_VILLAGER_BODY; 6],
            pitch_tracks_look: false,
        },
        // 7: Glowing amulet at the throat — the EMISSIVE part (index
        //    SATOSHI_GLOW_PART). Front of the upper chest (+Z, the face side).
        //    Small + warm — deliberately NOT the face (no creepy glowing eyes).
        ModelPart {
            origin: Vec3::new(0.0, 1.34, 0.17),
            size: Vec3::new(0.12, 0.12, 0.05),
            pivot: Vec3::ZERO,
            animated: false,
            phase: 0.0,
            tex_faces: [TEX_SATOSHI_TRIM; 6],
            pitch_tracks_look: false,
        },
    ]
}

/// Index of the emissive (self-glowing) part in [`satoshi_model`] — the throat
/// amulet. Its vertices are stamped with [`SATOSHI_GLOW_LIGHT`].
pub const SATOSHI_GLOW_PART: usize = 7;

/// Whether part `idx` of Satoshi's model self-glows (emissive). Pure.
pub fn satoshi_emissive_part(idx: usize) -> bool {
    idx == SATOSHI_GLOW_PART
}

/// Cached player-avatar model — built once, reused every frame (same pattern
/// as the mob models). Part order is fixed: head, body, left arm, right arm,
/// left leg, right leg. The head carries `pitch_tracks_look` so Phase 5 can
/// pitch it with the player's look direction; arms + legs animate on the
/// walk cycle in a diagonal gait (left arm + right leg share a phase, right
/// arm + left leg the other). Proportions mirror the villager humanoid mesh
/// with the player skin textures from `texture_gen`. Per-player tint lands in
/// Phase 6.
static PLAYER_MODEL: std::sync::LazyLock<Vec<ModelPart>> =
    std::sync::LazyLock::new(|| {
        vec![
            // Head — tracks look pitch. Pivot sits at the neck joint (bottom of
            // the head: origin.y - size.y/2 = 1.575 - 0.25 = 1.325) so look-pitch
            // reads as a nod about the neck, not a ball spinning in place.
            ModelPart {
                origin: Vec3::new(0.0, 1.575, 0.0),
                size: Vec3::new(0.5, 0.5, 0.5),
                pivot: Vec3::new(0.0, 1.325, 0.0),
                animated: false,
                phase: 0.0,
                tex_faces: [
                    TEX_PLAYER_HEAD_SIDE,  // index0 +X right
                    TEX_PLAYER_HEAD_SIDE,  // index1 -X left
                    TEX_PLAYER_HEAD_TOP,   // index2 +Y top
                    TEX_PLAYER_HEAD_TOP,   // index3 -Y bottom
                    TEX_PLAYER_HEAD_SIDE,  // index4 +Z (back of head)
                    TEX_PLAYER_HEAD_FRONT, // index5 -Z (front — the face, toward viewer)
                ],
                pitch_tracks_look: true,
            },
            // Body (torso) — stacks flush between the hip plane (0.625) and the
            // neck plane (1.325). Parts must never interpenetrate: buried bands
            // z-fight with custom skins (report 4519351d).
            ModelPart {
                origin: Vec3::new(0.0, 0.975, 0.0),
                size: Vec3::new(0.5, 0.7, 0.25),
                pivot: Vec3::ZERO,
                animated: false,
                phase: 0.0,
                tex_faces: [TEX_PLAYER_BODY; 6],
                pitch_tracks_look: false,
            },
            // Left arm — phase 0.0 (swings with right leg).
            ModelPart {
                origin: Vec3::new(-0.375, 1.05, 0.0),
                size: Vec3::new(0.25, 0.7, 0.25),
                pivot: Vec3::new(-0.375, 1.4, 0.0),
                animated: true,
                phase: 0.0,
                tex_faces: [TEX_PLAYER_ARM; 6],
                pitch_tracks_look: false,
            },
            // Right arm — phase 0.5 (swings with left leg).
            ModelPart {
                origin: Vec3::new(0.375, 1.05, 0.0),
                size: Vec3::new(0.25, 0.7, 0.25),
                pivot: Vec3::new(0.375, 1.4, 0.0),
                animated: true,
                phase: 0.5,
                tex_faces: [TEX_PLAYER_ARM; 6],
                pitch_tracks_look: false,
            },
            // Left leg — phase 0.5 (diagonal with right arm). Tops out flush at
            // the hip plane (0.625) where the torso begins; pivot = hip joint.
            ModelPart {
                origin: Vec3::new(-0.125, 0.3125, 0.0),
                size: Vec3::new(0.25, 0.625, 0.25),
                pivot: Vec3::new(-0.125, 0.625, 0.0),
                animated: true,
                phase: 0.5,
                tex_faces: [TEX_PLAYER_LEG; 6],
                pitch_tracks_look: false,
            },
            // Right leg — phase 0.0 (diagonal with left arm).
            ModelPart {
                origin: Vec3::new(0.125, 0.3125, 0.0),
                size: Vec3::new(0.25, 0.625, 0.25),
                pivot: Vec3::new(0.125, 0.625, 0.0),
                animated: true,
                phase: 0.0,
                tex_faces: [TEX_PLAYER_LEG; 6],
                pitch_tracks_look: false,
            },
        ]
    });

/// Minecraft's slim ("Alex") player model — [`PLAYER_MODEL`] with the two arms
/// narrowed from 4 px to 3 px and dropped half a pixel. Built once, like the
/// classic table, by patching rows 2 and 3 so head/body/legs have exactly one
/// definition and can never drift between the models.
///
/// The numbers come from [`crate::skin_pose`] (the shared geometry source): the
/// slim arm's box is `part_box`'s slim row, so a change there moves the mesh AND
/// the hit-test together instead of only one of them.
static PLAYER_MODEL_SLIM: std::sync::LazyLock<Vec<ModelPart>> =
    std::sync::LazyLock::new(|| {
        let mut parts = PLAYER_MODEL.clone();
        for (i, part) in parts.iter_mut().enumerate() {
            if i != 2 && i != 3 {
                continue;
            }
            let (min, max) =
                crate::skin_pose::part_box(i, crate::skin_uv::SkinLayer::Base, 0.0, false,
                    crate::skin_uv::ArmModel::Slim);
            part.origin = Vec3::new(
                (min[0] + max[0]) * 0.5,
                (min[1] + max[1]) * 0.5,
                (min[2] + max[2]) * 0.5,
            );
            part.size = Vec3::new(max[0] - min[0], max[1] - min[1], max[2] - min[2]);
            // The shoulder pivot travels with the box — Minecraft moves the
            // whole arm, rotation point included. It sits at the TOP-CENTRE of
            // the arm cuboid, which is where the classic table also puts it
            // (origin.y + size.y/2 = 1.05 + 0.35 = 1.4).
            part.pivot = Vec3::new(part.origin.x, max[1], part.pivot.z);
        }
        parts
    });

/// Humanoid player-avatar model parts (cached — no allocation after first
/// call). Part 0 is the head; see `PLAYER_MODEL` for layout details.
/// CLASSIC arms — [`player_model_for`] picks between the two models and is what
/// every render path calls; this shorthand is kept for the tests that pin the
/// classic layout (`skeleton::biped_matches_player_model_layout`).
#[cfg_attr(not(test), allow(dead_code))]
pub fn player_model() -> &'static [ModelPart] {
    PLAYER_MODEL.as_slice()
}

/// The player model for `arm`. One call, so no builder has to remember which
/// static to reach for.
pub fn player_model_for(arm: crate::skin_uv::ArmModel) -> &'static [ModelPart] {
    match arm {
        crate::skin_uv::ArmModel::Classic => PLAYER_MODEL.as_slice(),
        crate::skin_uv::ArmModel::Slim => PLAYER_MODEL_SLIM.as_slice(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Workshop mannequin builder (`build_workshop_avatar_vertices`) relies
    /// on `ps.yaw = −π/2` + a zero position making this builder emit vertices in
    /// MODEL space — the space `skin_pose::part_box` and `skin_hit` work in — so
    /// that ONE shared transform (`skin_grid::avatar_model_to_world`, the exact
    /// inverse of the paint ray's `workshop::world_ray_to_avatar_model`) puts
    /// them in the world. If the builder's internal `yaw + π/2` convention ever
    /// changes, the mannequin would silently rotate away from the crosshair;
    /// this catches it here instead.
    #[test]
    fn workshop_mannequin_builds_in_model_space_before_the_shared_transform() {
        let registry = crate::block::BlockRegistry::new();
        let ps = crate::protocol::PlayerState {
            player_index: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: -std::f32::consts::FRAC_PI_2,
            pitch: 0.0,
            health: 20.0,
            held_kind: 0,
            held_id: 0,
            anim_state: 0,
            flags: 0,
            skin_key: 0,
        };
        let opts = SkinRenderOpts { separation: 1.0, edit_inflate: true, ..Default::default() };
        let (verts, _held) =
            build_player_avatar_vertices_with(&ps, 0.0, 0, 0, 1.0, &registry, opts);
        assert!(!verts.is_empty(), "the mannequin must produce geometry");

        let (lo, hi) = crate::skin_pose::avatar_aabb(1.0, crate::skin_pose::EDIT_INFLATE);
        let mut vlo = [f32::INFINITY; 3];
        let mut vhi = [f32::NEG_INFINITY; 3];
        for v in &verts {
            for a in 0..3 {
                vlo[a] = vlo[a].min(v.position[a]);
                vhi[a] = vhi[a].max(v.position[a]);
            }
        }
        for a in 0..3 {
            assert!(
                vlo[a] >= lo[a] - 1e-4 && vhi[a] <= hi[a] + 1e-4,
                "axis {a}: verts span {}..{} but the model-space box is {}..{}",
                vlo[a], vhi[a], lo[a], hi[a]
            );
        }
        // …and it FILLS that box on x — a build left in world orientation would
        // put the separated arms on z instead, so this is what pins the
        // "no rotation applied yet" claim rather than merely bounding it.
        assert!((vlo[0] - lo[0]).abs() < 1e-3, "left arm should reach x = {}", lo[0]);
        assert!((vhi[0] - hi[0]).abs() < 1e-3, "right arm should reach x = {}", hi[0]);
        // …and on y, where the separated legs drop to the bottom of the box and
        // the raised head reaches its top.
        assert!((vlo[1] - lo[1]).abs() < 1e-3, "legs should reach y = {}", lo[1]);
        assert!((vhi[1] - hi[1]).abs() < 1e-3, "head should reach y = {}", hi[1]);
    }

    #[test]
    fn remote_items_render_as_drop_cubes_and_skip_unknowns() {
        // Death-drops phase 2b — server-broadcast items render exactly like
        // local ItemEntity drops (one 6-face cube each), resolved from the
        // wire ItemRef; hostile/unknown refs are skipped, never AIR-cubed.
        use crate::protocol::{EntityKind, EntitySpawn, ItemRef};
        let registry = crate::block::BlockRegistry::new();
        let mut items = crate::remote_entities::RemoteItems::default();
        let spawn = |id: u32, item: ItemRef, x: f32| {
            let (item_kind, item_id) = item.to_wire();
            EntitySpawn {
                id,
                kind: EntityKind::Item,
                x,
                y: 65.0,
                z: 0.0,
                yaw: 0.0,
                health: 0,
                item_kind,
                item_id,
                item_count: 1,
                full_item: crate::protocol::WireItem::None,
            }
        };
        items.apply(
            &[
                spawn(1, ItemRef::Block(crate::block::STONE), 4.0),
                spawn(2, ItemRef::Material(crate::item::MaterialId::Bone as u16), 8.0),
                spawn(3, ItemRef::Tool(2), 12.0),
                // Unknown block id — must render nothing (the AIR fallback
                // would draw a ghost cube).
                spawn(4, ItemRef::Block(u16::MAX), 16.0),
                // Unknown material discriminant — likewise skipped.
                spawn(5, ItemRef::Material(u16::MAX), 20.0),
            ],
            &[],
            &[],
        );
        let verts = build_remote_item_vertices(
            &items,
            &registry,
            0,
            glam::Vec3::new(-10.0, 65.0, 0.0),
            |_| (1.0, 0.0),
        );
        assert_eq!(
            verts.len(),
            3 * 36,
            "block + material + tool render one cube each; unknowns skipped"
        );
    }

    #[test]
    fn remote_projectiles_render_as_the_same_arrow_as_local_ones() {
        // MP-A3 — a server-shot arrow draws the very cuboid a local arrow
        // does (`push_arrow` is shared); one too close to the eye is skipped.
        use crate::protocol::{EntityKind, EntitySpawn, WireItem};
        let mut arrows = crate::remote_entities::RemoteProjectiles::default();
        let spawn = |id: u32, x: f32| EntitySpawn {
            id,
            kind: EntityKind::Projectile,
            x,
            y: 65.0,
            z: 0.0,
            yaw: 0.0,
            health: 0,
            item_kind: 0,
            item_id: 0,
            item_count: 0,
            full_item: WireItem::None,
        };
        arrows.apply(&[spawn(1, 4.0), spawn(2, 8.0), spawn(3, -10.0)], &[], &[]);
        let eye = glam::Vec3::new(-10.0, 65.0, 0.0);
        let remote = build_remote_projectile_vertices(&arrows, eye, |_| (1.0, 0.0));

        let mut ecs = hecs::World::new();
        crate::entity::spawn_arrow(&mut ecs, glam::Vec3::new(4.0, 65.0, 0.0), glam::Vec3::new(0.0, 0.0, -1.0), 4.0, None);
        let local = build_projectile_vertices(&ecs, eye, |_| (1.0, 0.0));
        assert!(!local.is_empty());
        assert_eq!(remote.len(), 2 * local.len(), "two arrows drawn; the one at the eye skipped");
    }

    #[test]
    fn remote_armour_and_tool_render_from_the_full_payload() {
        // Death-drops phase 3 — armour's legacy pair is `Empty`, so it used
        // to render nothing at all. With `full_item` it draws the same cube a
        // LOCAL armour drop does (item_textures is shared by both paths).
        use crate::armour::{ArmourItem, ArmourMaterial, ArmourSlot};
        use crate::crafting::{Tool, ToolMaterial, ToolType};
        use crate::protocol::{EntityKind, EntitySpawn};
        let registry = crate::block::BlockRegistry::new();
        let mut items = crate::remote_entities::RemoteItems::default();
        let spawn = |id: u32, item: &crate::item::Item, x: f32| {
            let (item_kind, item_id) = crate::inventory::item_to_ref(item).to_wire();
            EntitySpawn {
                id,
                kind: EntityKind::Item,
                x,
                y: 65.0,
                z: 0.0,
                yaw: 0.0,
                health: 0,
                item_kind,
                item_id,
                item_count: 1,
                full_item: crate::inventory::item_to_wire_full(item),
            }
        };
        let mut pick = Tool::new(ToolType::Pickaxe, ToolMaterial::Iron);
        pick.durability = 37;
        let boots = ArmourItem::new(ArmourSlot::Boots, ArmourMaterial::Diamond);
        items.apply(
            &[
                spawn(1, &crate::item::Item::Tool(pick), 4.0),
                spawn(2, &crate::item::Item::Armour(boots), 8.0),
            ],
            &[],
            &[],
        );
        let verts = build_remote_item_vertices(
            &items,
            &registry,
            0,
            glam::Vec3::new(-10.0, 65.0, 0.0),
            |_| (1.0, 0.0),
        );
        assert_eq!(verts.len(), 2 * 36, "tool + armour each render one drop cube");
    }

    #[test]
    fn workshop_inworld_renders_parked_projects_scaled() {
        // Spec 40 — the in-world inflated mannequin render: a parked block project
        // emits a scaled cube (36 verts), a parked mob project emits its scaled
        // model, and a PINNED project emits nothing (it's already applied globally).
        use crate::workshop::{WorkshopMode, WorkshopTarget};
        let registry = crate::block::BlockRegistry::new();
        let mut world = crate::world::World::new();

        // A block project, inflated.
        let b = world.workshop.add(WorkshopTarget::Block(crate::block::STONE), WorkshopMode::Reskin, [0, 79, 0]);
        world.workshop.get_mut(b).unwrap().pump();

        let v = build_workshop_inworld_vertices(&world, &registry, 0.0, false);
        assert_eq!(v.len(), 36, "a block mannequin is one cube = 6 faces × 6 verts");

        // A larger inflation spreads the cube wider (higher = bigger).
        for _ in 0..4 {
            world.workshop.get_mut(b).unwrap().pump();
        }
        let v_big = build_workshop_inworld_vertices(&world, &registry, 0.0, false);
        let span = |vs: &[Vertex]| {
            let xs: Vec<f32> = vs.iter().map(|v| v.position[0]).collect();
            xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!(span(&v_big) > span(&v), "more inflation → bigger mannequin");

        // Pin it → no longer rendered as a working copy.
        world.workshop.get_mut(b).unwrap().pin();
        assert!(
            build_workshop_inworld_vertices(&world, &registry, 0.0, false).is_empty(),
            "pinned projects are not rendered in-world"
        );

        // A mob project emits its multi-part model.
        let m = world.workshop.add(WorkshopTarget::Mob(crate::mob::MobType::Cow), WorkshopMode::Reskin, [3, 79, 0]);
        world.workshop.get_mut(m).unwrap().pump();
        let vm = build_workshop_inworld_vertices(&world, &registry, 0.0, false);
        let parts = mob_model(crate::mob::MobType::Cow).len();
        assert_eq!(vm.len(), parts * 36, "mob mannequin = one cube per model part");
    }

    #[test]
    fn avatar_light_rides_sky_channel_and_preserves_fade() {
        // Campaign N — avatar verts carry combined light in `sky_light`; the
        // `light` channel is the Phase-3 fade alpha and must never be touched.
        let mut verts = vec![Vertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            tex_layer: 0,
            uv: [0.0, 0.0],
            light: 0.4, // a mid-fade value
            sky_light: 1.0,
        }];
        set_avatar_light(&mut verts, 0.3);
        assert_eq!(verts[0].sky_light, 0.3);
        assert_eq!(verts[0].light, 0.4, "fade alpha untouched");
        // combined = max(block, sky * sky_brightness) — same formula as fs_main.
        assert!((combined_light((0.2, 1.0), 0.5) - 0.5).abs() < 1e-6);
        assert!((combined_light((0.8, 1.0), 0.5) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn entity_builder_bakes_sampled_light_channels() {
        // Campaign N (2026-07-05) — entities sample world light so they dim at
        // night and in caves like terrain does. The per-entity sample
        // overwrites the emitters' FULL_BRIGHT bake; emissive verts (Satoshi's
        // glow sentinel) and the GlowSquid (the game's living lamp) stay lit.
        use crate::combat::Health;
        use crate::entity::{Hitbox, MobKind, Position, Velocity};
        use crate::mob::MobType;
        use crate::mob_ai::MobAi;
        use crate::override_registry::OverrideRegistry;

        let far = Vec3::new(10.0, 0.0, 0.0);
        let empty = OverrideRegistry::new();
        let spawn = |kind: MobType| {
            let mut ecs = hecs::World::new();
            ecs.spawn((
                Position(Vec3::ZERO),
                Velocity(Vec3::ZERO),
                MobKind(kind),
                Hitbox { width: 0.9, height: 1.3 },
                MobAi::new(),
                Health::new(10.0),
            ));
            ecs
        };

        // A cow in a dim cell carries the sampled channels on every vertex.
        let cow = spawn(MobType::Cow);
        let verts = build_entity_model_vertices(&cow, 0, far, &empty, |_| (0.2, 0.5));
        assert!(!verts.is_empty());
        assert!(
            verts.iter().all(|v| (v.light - 0.2).abs() < 1e-6 && (v.sky_light - 0.5).abs() < 1e-6),
            "every cow vertex carries the sampled (block, sky) channels"
        );

        // GlowSquid stays full-bright regardless of the sample.
        let squid = spawn(MobType::GlowSquid);
        let sv = build_entity_model_vertices(&squid, 0, far, &empty, |_| (0.1, 0.0));
        assert!(!sv.is_empty());
        assert!(
            sv.iter().all(|v| v.light == Vertex::FULL_BRIGHT),
            "GlowSquid is exempt — the living lamp never dims"
        );

        // Satoshi: his amulet keeps the emissive sentinel (> 1.5) while the
        // rest of him takes the sample.
        let mut s = spawn(MobType::Villager);
        let e = s.iter().next().map(|e| e.entity()).unwrap();
        s.insert_one(e, crate::satoshi::SatoshiMarker).unwrap();
        let satv = build_entity_model_vertices(&s, 0, far, &empty, |_| (0.2, 0.0));
        assert!(satv.iter().any(|v| v.light > 1.5), "glow sentinel survives the light bake");
        assert!(
            satv.iter().any(|v| (v.light - 0.2).abs() < 1e-6),
            "non-emissive Satoshi parts take the sample"
        );
    }

    #[test]
    fn mob_appearance_override_retextures_part_at_entity_builder() {
        // Spec 40 (The Workshop) Phase A — a registered mob-part reskin re-textures
        // that part's faces when `build_entity_model_vertices` emits the mob, while
        // an un-overridden mob renders byte-identically. This exercises the SECOND
        // override seam (the entity vertex builder), complementing the mesher test.
        use crate::combat::Health;
        use crate::entity::{Hitbox, MobKind, Position, Velocity};
        use crate::mob::MobType;
        use crate::mob_ai::MobAi;
        use crate::override_registry::{AuthoredFaces, MobPartKey, NamedDesign, OverrideRegistry};

        let base = crate::texture_gen::texture_count();

        // One cow at the origin; camera far away so it isn't culled (dist² > 0.25).
        let mut ecs = hecs::World::new();
        ecs.spawn((
            Position(Vec3::ZERO),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Cow),
            Hitbox { width: 0.9, height: 1.3 },
            MobAi::new(),
            Health::new(10.0),
        ));
        let far = Vec3::new(10.0, 0.0, 0.0);

        // Baseline: no overrides → every vertex uses a stock layer (< base).
        let empty = OverrideRegistry::new();
        let before = build_entity_model_vertices(&ecs, 0, far, &empty, |_| (1.0, 0.0));
        assert!(!before.is_empty(), "a visible cow should emit vertices");
        assert!(
            before.opaque_layers_all_below(base),
            "unoverridden cow must use stock texture layers"
        );

        // Reskin the cow's first part (the body). Solid ⇒ all six body faces map
        // to the single appended layer at `base`.
        let mut overrides = OverrideRegistry::new();
        overrides.add_mob_design(
            MobPartKey { mob: MobType::Cow, part: 0 },
            NamedDesign {
                id: 0,
                name: "test".into(),
                faces: Some(AuthoredFaces::solid([12, 200, 60, 255])),
                micro_model: None,
                author_npub: String::new(),
                derivation_chain: vec![],
            },
            base,
        );
        let after = build_entity_model_vertices(&ecs, 0, far, &overrides, |_| (1.0, 0.0));
        assert!(
            after.iter().any(|v| v.tex_layer == base),
            "the reskinned body part must emit the override layer ({base})"
        );
        assert!(
            after.iter().any(|v| v.tex_layer < base),
            "the cow's other (un-reskinned) parts must keep stock layers"
        );
    }

    // Small assertion helper kept local to the test to avoid leaking a method.
    trait LayersBelow {
        fn opaque_layers_all_below(&self, base: u32) -> bool;
    }
    impl LayersBelow for Vec<Vertex> {
        fn opaque_layers_all_below(&self, base: u32) -> bool {
            self.iter().all(|v| v.tex_layer < base)
        }
    }

    #[test]
    fn satoshi_model_is_distinct_and_has_a_glow_part() {
        // The cloaked sage has the 6 villager parts PLUS a cloak drape and a
        // glowing amulet — and only the amulet self-glows.
        let s = satoshi_model();
        assert!(
            s.len() > villager_model().len(),
            "Satoshi has extra parts (cloak drape + amulet) beyond the villager"
        );
        assert!(satoshi_emissive_part(SATOSHI_GLOW_PART), "the amulet part self-glows");
        assert!(!satoshi_emissive_part(0), "the torso does not glow");
        assert!(SATOSHI_GLOW_PART < s.len(), "the glow part index is in range");
    }

    #[test]
    fn satoshi_marker_swaps_the_model_and_stamps_the_glow() {
        // The SatoshiMarker makes build_entity_model_vertices render his hooded
        // model and stamp the amulet's emissive sentinel — a plain villager does
        // neither.
        use crate::combat::Health;
        use crate::entity::{Hitbox, MobKind, Position, Velocity};
        use crate::mob::MobType;
        use crate::mob_ai::MobAi;
        use crate::override_registry::OverrideRegistry;

        let far = Vec3::new(10.0, 0.0, 0.0);
        let empty = OverrideRegistry::new();

        // Plain villager: no glow sentinel, no Satoshi textures.
        let mut plain = hecs::World::new();
        plain.spawn((
            Position(Vec3::ZERO),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Villager),
            Hitbox { width: 0.6, height: 1.9 },
            MobAi::new(),
            Health::new(20.0),
        ));
        let pv = build_entity_model_vertices(&plain, 0, far, &empty, |_| (1.0, 0.0));
        assert!(!pv.is_empty(), "a visible villager emits vertices");
        assert!(pv.iter().all(|v| v.light < 1.5), "a plain villager emits no glow sentinel");
        assert!(
            pv.iter().all(|v| v.tex_layer != TEX_SATOSHI_HEAD_FRONT),
            "a plain villager never uses Satoshi's hooded-head texture"
        );

        // Satoshi: same entity + marker → his model + the amulet glow.
        let mut s = hecs::World::new();
        let id = s.spawn((
            Position(Vec3::ZERO),
            Velocity(Vec3::ZERO),
            MobKind(MobType::Villager),
            Hitbox { width: 0.6, height: 1.9 },
            MobAi::new(),
            Health::new(20.0),
        ));
        let _ = s.insert_one(id, crate::satoshi::SatoshiMarker);
        let sv = build_entity_model_vertices(&s, 0, far, &empty, |_| (1.0, 0.0));
        assert!(
            sv.iter().any(|v| v.light > 1.5),
            "Satoshi's amulet emits the glow sentinel"
        );
        assert!(
            sv.iter().any(|v| v.tex_layer == TEX_SATOSHI_HEAD_FRONT),
            "Satoshi renders his hooded head"
        );
    }

    #[test]
    fn every_passive_spawn_mob_has_a_model() {
        // Regression (2026-05-27): Horse/Rabbit/Wolf/Bee/Squid/Goat/
        // Nostrich spawned via scatter_mobs_in_column but had no entry in
        // MODEL_CACHE, so mob_model() returned `&[]` and they rendered as
        // nothing — invisible animals. Any mob the world can spawn MUST
        // have a non-empty model. Sweep every biome's passive table.
        use crate::biome::Biome;
        let biomes = [
            Biome::Plains, Biome::Forest, Biome::BirchForest, Biome::Taiga,
            Biome::Jungle, Biome::Savanna, Biome::Desert, Biome::SnowyTundra,
            Biome::Mountains, Biome::Ocean,
        ];
        for b in biomes {
            for (kind, _weight) in crate::mob::biome_passive_spawn_weights(b) {
                assert!(
                    !mob_model(kind).is_empty(),
                    "{:?} can spawn in {:?} but has no model — it would be invisible",
                    kind, b,
                );
            }
        }
    }

    #[test]
    fn player_model_shape() {
        let parts = player_model();
        assert_eq!(parts.len(), 6); // head, body, arm_l, arm_r, leg_l, leg_r
        assert!(parts[0].pitch_tracks_look); // head tracks look pitch
        let animated: Vec<f32> = parts.iter().filter(|p| p.animated).map(|p| p.phase).collect();
        assert_eq!(animated.len(), 4); // two arms + two legs
        assert!(animated.contains(&0.0) && animated.contains(&0.5)); // alternating phase
    }

    #[test]
    fn player_model_parts_do_not_interpenetrate() {
        // Report 4519351d ("overlap at the body and the legs"): the parts used
        // to be squashed into each other (legs 2px up into the torso, head into
        // the chest) with coplanar same-facing faces, which z-fights wherever a
        // custom skin paints the buried band a different colour — glaring on
        // the Workshop's ×4 mannequin. Parts may touch at joint planes but must
        // never overlap in volume.
        let parts = player_model();
        for i in 0..parts.len() {
            for j in (i + 1)..parts.len() {
                let (a, b) = (&parts[i], &parts[j]);
                let (amin, amax) = (a.origin - a.size * 0.5, a.origin + a.size * 0.5);
                let (bmin, bmax) = (b.origin - b.size * 0.5, b.origin + b.size * 0.5);
                let overlap = (amax.min(bmax) - amin.max(bmin)).max(Vec3::ZERO);
                assert!(
                    overlap.min_element() <= 1e-6,
                    "parts {i} and {j} interpenetrate: overlap {overlap:?}"
                );
            }
        }
        // And the trunk stacks flush — no gap at the hips or the neck either.
        let (head, body, leg) = (&parts[0], &parts[1], &parts[4]);
        let hip = body.origin.y - body.size.y * 0.5 - (leg.origin.y + leg.size.y * 0.5);
        let neck = head.origin.y - head.size.y * 0.5 - (body.origin.y + body.size.y * 0.5);
        assert!(hip.abs() < 1e-6, "hip joint must be flush, gap {hip}");
        assert!(neck.abs() < 1e-6, "neck joint must be flush, gap {neck}");
    }

    #[test]
    fn swing_curve_bounds() {
        assert!(swing_angle(0.0).abs() < 1e-6);
        assert!(swing_angle(1.0).abs() < 1e-6);
        // Sweep the window: strictly positive interior, and the true maximum of
        // (1-t)·sin(πt) is ~0.579 (near t≈0.355), so the peak sits at ~0.58·SWING_PEAK.
        // A tight band around that catches a doubled/halved/wrong formula.
        let mut max = 0.0f32;
        for i in 1..100 {
            let v = swing_angle(i as f32 / 100.0);
            assert!(v > 0.0, "interior must be positive");
            max = max.max(v);
        }
        assert!(max < SWING_PEAK * 0.62, "peak {max} should be ~0.58·SWING_PEAK");
        assert!(max > SWING_PEAK * 0.55, "peak {max} unexpectedly low");
    }

    fn test_part(pitch_tracks_look: bool) -> ModelPart {
        ModelPart {
            origin: Vec3::new(0.0, 1.0, 0.0),
            size: Vec3::new(0.5, 0.5, 0.5),
            pivot: Vec3::new(0.0, 0.75, 0.0),
            animated: false,
            phase: 0.0,
            tex_faces: [0; 6],
            pitch_tracks_look,
        }
    }

    fn part_verts(part: &ModelPart, pose: PartPose) -> Vec<Vertex> {
        let mut v = Vec::new();
        build_part_vertices(&mut v, part, Vec3::ZERO, 0.0, pose, None, 1.0);
        v
    }

    fn positions_differ(a: &[Vertex], b: &[Vertex]) -> bool {
        a.len() != b.len()
            || a.iter().zip(b).any(|(x, y)| {
                x.position
                    .iter()
                    .zip(y.position)
                    .any(|(p, q)| (p - q).abs() > 1e-6)
            })
    }

    #[test]
    fn head_pitch_moves_a_pitch_tracking_part() {
        let head = test_part(true);
        let neutral = part_verts(&head, PartPose::default());
        let pitched = part_verts(&head, PartPose { head_pitch: 0.5, ..PartPose::default() });
        assert!(positions_differ(&neutral, &pitched));
    }

    #[test]
    fn head_pitch_ignored_by_non_head_part() {
        let body = test_part(false);
        let neutral = part_verts(&body, PartPose::default());
        let pitched = part_verts(&body, PartPose { head_pitch: 0.5, ..PartPose::default() });
        assert!(!positions_differ(&neutral, &pitched));
    }

    fn avatar_state(held_kind: u8, held_id: u16, flags: u8) -> crate::protocol::PlayerState {
        crate::protocol::PlayerState {
            player_index: 1,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.3,
            health: 20.0,
            held_kind,
            held_id,
            anim_state: 1, // walking
            flags,
            skin_key: 0, // avatar-model tests don't exercise the skin reference
        }
    }

    #[test]
    fn push_skin_quad_winding_matches_textured() {
        // Same vertex ORDER and winding as push_textured_quad, but the u ends are
        // swapped: a=br→[u1,v1], b=bl→[u0,v1], c=tl→[u0,v0], d=tr→[u1,v0].
        // That swap is the Minecraft box unwrap — see push_skin_quad's docs and
        // skin_faces_follow_the_minecraft_box_unwrap. 6 verts/quad.
        let mut v = Vec::new();
        let rect = [0.1_f32, 0.2, 0.3, 0.4]; // [u0,v0,u1,v1]
        push_skin_quad(
            &mut v,
            [0.0, 0.0, -1.0],
            Vec3::new(0.0, 0.0, 0.0), // a
            Vec3::new(1.0, 0.0, 0.0), // b
            Vec3::new(1.0, 1.0, 0.0), // c
            Vec3::new(0.0, 1.0, 0.0), // d
            rect,
            0,
        );
        assert_eq!(v.len(), 6, "a textured quad is two triangles = 6 verts");
        // Vertex 0 is corner a → (u1, v1).
        assert_eq!(v[0].uv, [0.3, 0.4], "corner a maps to (u1, v1)");
        // Vertex 2 is corner c → (u0, v0).
        assert_eq!(v[2].uv, [0.1, 0.2], "corner c maps to (u0, v0)");
        // Vertex 5 is corner d → (u1, v0).
        assert_eq!(v[5].uv, [0.3, 0.2], "corner d maps to (u1, v0)");
        // tex_layer is forwarded from the layer argument (0 here).
        assert!(v.iter().all(|x| x.tex_layer == 0));
    }

    #[test]
    fn skin_faces_follow_the_minecraft_box_unwrap() {
        // The classic 64x64 layout is an UNROLLED cube: the band at y=8..16 runs
        // right | front | left | back across x=0..32, so neighbouring tiles share
        // real box edges. That adjacency pins each face's u DIRECTION, not just
        // which tile it samples:
        //   - the right-side tile's u1 edge joins the front tile → u1 = front
        //   - the front tile's u0 edge joins the right-side tile → u0 = the
        //     character's right (+X)
        //   - the left-side tile's u0 edge joins the front tile  → u0 = front
        // Get this backwards and every face renders mirrored about its own
        // vertical axis: an imported Minecraft skin reads back-to-front on the
        // sides of the head, and text on the face comes out reversed.
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let mut ps = avatar_state(item_kind::EMPTY, 0, 0);
        ps.pitch = 0.0; // the head tracks look; keep the box axis-aligned
        ps.anim_state = 0; // no walk swing
        // Run the whole invariant on BOTH player models. Slim narrows the arm
        // tiles AND the arm box; if only one of the two moved, the arm seams
        // below stop lining up even though the head ones still do.
        for arm_model in [crate::skin_uv::ArmModel::Classic, crate::skin_uv::ArmModel::Slim] {
        let (v, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 0, 0, 1.0, &reg,
            SkinRenderOpts { draw_overlay: false, arm_model, ..Default::default() },
        );
        let table = crate::skin_uv::base_faces(arm_model);

        // Pick a face by its atlas rect (unique per base face), matching whole
        // quads — vertices land on rect corners, and neighbouring tiles touch at
        // those corners, so a per-vertex filter would over-collect. Faces are
        // emitted 6 verts at a time. This needs no assumption about emission
        // order OR about which way the avatar is turned in world space (the
        // vertices come out yawed).
        let face = |rect: [f32; 4]| -> Vec<&Vertex> {
            let in_rect = |x: &Vertex| {
                x.uv[0] >= rect[0] - 1e-6
                    && x.uv[0] <= rect[2] + 1e-6
                    && x.uv[1] >= rect[1] - 1e-6
                    && x.uv[1] <= rect[3] + 1e-6
            };
            v.chunks(6)
                .find(|q| q.iter().all(in_rect))
                .unwrap_or_else(|| panic!("no quad found for atlas rect {rect:?}"))
                .iter()
                .collect()
        };
        // Two faces of a box share a physical edge: the vertices they have in
        // common. Assert the u each side carries THERE — that is exactly the
        // unwrap invariant, and it is invariant under any rotation of the model.
        let assert_shared_edge =
            |a: &[&Vertex], a_u: f32, b: &[&Vertex], b_u: f32, what: &str| {
                let same = |p: [f32; 3], q: [f32; 3]| {
                    (0..3).all(|i| (p[i] - q[i]).abs() < 1e-5)
                };
                let mut found = 0;
                for va in a {
                    if !b.iter().any(|vb| same(va.position, vb.position)) {
                        continue;
                    }
                    found += 1;
                    assert!(
                        (va.uv[0] - a_u).abs() < 1e-6,
                        "{what}: expected u {a_u} on the shared edge, got {}",
                        va.uv[0]
                    );
                    for vb in b.iter().filter(|vb| same(va.position, vb.position)) {
                        assert!(
                            (vb.uv[0] - b_u).abs() < 1e-6,
                            "{what}: the neighbouring tile expected u {b_u} on the \
                             shared edge, got {}",
                            vb.uv[0]
                        );
                    }
                }
                assert!(found >= 2, "{what}: the two faces share no edge");
            };

        // Every part unrolls the same way, so run the seam check on the head
        // (8-wide tiles, unchanged by slim) AND on both arms (the tiles slim
        // actually narrows). Strip order is [side | front | side | back], i.e.
        // face indices 0, 5, 1, 4.
        for (part, what) in [(0usize, "head"), (2, "left arm"), (3, "right arm")] {
            let p = table[part];
            let first = face(p[0]);
            let front = face(p[5]);
            let second = face(p[1]);
            assert_eq!(first.len(), 6, "{what} {arm_model:?}: first side tile is one quad");
            assert_eq!(front.len(), 6, "{what} {arm_model:?}: front tile is one quad");
            assert_eq!(second.len(), 6, "{what} {arm_model:?}: second side tile is one quad");

            // side | front: the side tile's u1 against the front tile's u0.
            // Walking around the box must walk FORWARD along the atlas.
            assert_shared_edge(
                &first,
                p[0][2],
                &front,
                p[5][0],
                &format!("{what} {arm_model:?} side|front seam"),
            );
            // front | side: front's u1 against the next tile's u0.
            assert_shared_edge(
                &front,
                p[5][2],
                &second,
                p[1][0],
                &format!("{what} {arm_model:?} front|side seam"),
            );
        }
        }
    }

    #[test]
    fn clothes_off_drops_the_overlay_boxes() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        let (with, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 10, 0, 1.0, &reg,
            SkinRenderOpts { draw_overlay: true, ..Default::default() },
        );
        let (without, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 10, 0, 1.0, &reg,
            SkinRenderOpts { draw_overlay: false, ..Default::default() },
        );
        // 6 parts × 6 faces × 6 verts = 216 verts per layer.
        assert_eq!(with.len(), 432, "base + overlay");
        assert_eq!(without.len(), 216, "base only");
    }

    #[test]
    fn separation_moves_the_arm_but_not_the_body() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        let (together, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 10, 0, 1.0, &reg, SkinRenderOpts::default(),
        );
        let (apart, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 10, 0, 1.0, &reg,
            SkinRenderOpts { separation: 1.0, ..Default::default() },
        );
        assert_eq!(together.len(), apart.len(), "same vertex count either way");
        // Horizontal radius from the model axis — invariant under the avatar's
        // yaw, unlike a single-axis span (the builder rotates by ps.yaw + PI/2,
        // so model X does not map to world X).
        let span = |v: &[Vertex]| {
            v.iter().fold(0.0f32, |m, vert| {
                let (x, z) = (vert.position[0], vert.position[2]);
                m.max((x * x + z * z).sqrt())
            })
        };
        assert!(
            span(&apart) > span(&together) + 0.2,
            "apart pose must widen the silhouette: {} vs {}",
            span(&apart),
            span(&together)
        );
    }

    #[test]
    fn default_opts_match_the_legacy_builder() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        let (legacy, _) = build_player_avatar_vertices(&ps, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        let (opted, _) = build_player_avatar_vertices_with(
            &ps, 0.0, 10, 0, 1.0, &reg, SkinRenderOpts::default(),
        );
        assert_eq!(legacy.len(), opted.len());
        for (a, b) in legacy.iter().zip(opted.iter()) {
            assert_eq!(a.position, b.position, "default opts must not change the worn avatar");
        }
    }

    #[test]
    fn avatar_skin_is_base_plus_overlay_held_item_separate() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        // Body skin = 6 parts × 2 layers (base+overlay) × 6 faces × 6 verts
        // = 12 boxes × 36 = 432 verts, all in the FIRST tuple element.
        let (skin, held) = build_player_avatar_vertices(
            &avatar_state(item_kind::EMPTY, 0, 0),
            0.0,
            10,
            0,
            1.0,
            &reg,
            crate::skin_uv::ArmModel::Classic,
        );
        assert_eq!(skin.len(), 432, "6 parts × (base+overlay) = 12 boxes -> 432 verts");
        assert!(held.is_empty(), "no held item -> empty held mesh");

        // With a held tool the body skin is unchanged; the item sub-mesh
        // (a 36-vert cuboid) lands in the SECOND tuple element, never the skin.
        let (skin2, held2) = build_player_avatar_vertices(
            &avatar_state(item_kind::TOOL, 2, 0),
            0.0,
            10,
            0,
            1.0,
            &reg,
            crate::skin_uv::ArmModel::Classic,
        );
        assert_eq!(skin2.len(), 432, "held item must not change the skin mesh");
        assert_eq!(held2.len(), 36, "held tool is a 36-vert cuboid in .1, not .0");
    }

    // ── Third-person camera — Phase 1, Task 4 (L2 vertex-presence) ───────────
    // The local player's OWN avatar is drawn in third-person and hidden in
    // first-person (where the viewmodel hand draws instead). Pure + GPU-free.

    #[test]
    fn self_avatar_present_in_third_person_empty_in_first() {
        use crate::camera::CameraMode;
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);

        // First-person: no self-avatar body (the first-person hand draws instead).
        let (skin_fp, held_fp) = self_avatar_vertices(CameraMode::FirstPerson, &ps, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(skin_fp.is_empty(), "first-person draws no self-avatar body");
        assert!(held_fp.is_empty(), "first-person self-avatar carries no held mesh");

        // Third-person: the full body mesh (same 432 verts a remote peer draws).
        for mode in [CameraMode::OverShoulder, CameraMode::OrbitBehind] {
            let (skin, _held) = self_avatar_vertices(mode, &ps, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
            assert_eq!(skin.len(), 432, "third-person draws the player's own body ({mode:?})");
        }
    }

    #[test]
    fn self_avatar_carries_the_players_held_item_in_third_person() {
        use crate::camera::CameraMode;
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        // Holding a tool → the held sub-mesh appears in third-person (.1), like a remote.
        let ps = avatar_state(item_kind::TOOL, 2, 0);
        let (_skin, held) = self_avatar_vertices(CameraMode::OverShoulder, &ps, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert_eq!(held.len(), 36, "third-person self-avatar shows the held tool");
    }

    // ── Third-person camera — Phase 3 (L2: avatar verts carry the fade alpha) ─
    // The self-avatar fade rides in the per-vertex `light` channel (always
    // full-bright for avatars, so it's a free carrier). `fs_avatar` reads it as a
    // screen-door dither alpha. fade 1.0 → opaque; <1.0 → every skin vert tagged.

    #[test]
    fn self_avatar_verts_carry_the_fade_alpha_in_the_light_channel() {
        use crate::camera::CameraMode;
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        // Opaque (fade 1.0): every skin vert is full-bright.
        let (opaque, _) = self_avatar_vertices(CameraMode::OrbitBehind, &ps, 0.0, 0, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(!opaque.is_empty(), "third-person builds self-avatar geometry");
        assert!(
            opaque.iter().all(|v| (v.light - 1.0).abs() < 1e-6),
            "fade 1.0 → full-bright (light==1.0) on every skin vert"
        );
        // Faded (0.5): every skin vert carries the fade alpha in its light channel.
        let (faded, _) = self_avatar_vertices(CameraMode::OrbitBehind, &ps, 0.0, 0, 0, 0.5, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(!faded.is_empty());
        assert!(
            faded.iter().all(|v| (v.light - 0.5).abs() < 1e-6),
            "fade 0.5 → light==0.5 on every skin vert (the dither alpha)"
        );
    }

    #[test]
    fn first_person_self_avatar_is_empty_regardless_of_fade() {
        use crate::camera::CameraMode;
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        let (v, _) = self_avatar_vertices(CameraMode::FirstPerson, &ps, 0.0, 0, 0, 0.3, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(v.is_empty(), "first-person draws no self-avatar, faded or not");
    }

    #[test]
    fn remote_avatar_verts_are_full_bright_with_fade_one() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 1, 0);
        // Remote/split-screen avatars never fade — they pass fade 1.0.
        let (v, _) = build_player_avatar_vertices(&ps, 0.0, 0, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(
            v.iter().all(|x| (x.light - 1.0).abs() < 1e-6),
            "remote avatars are always full-bright (no self-fade)"
        );
    }

    // ── Third-person camera — facing fix (2026-06-09 playtest) ───────────────
    // The self-avatar must face where the camera LOOKS (so you see the back of
    // the head and it tracks, not counter-rotates). yaw_facing converts a look
    // direction to the entity-convention model yaw; avatar_front_dir is its
    // inverse, pinned to build_skin_part_vertices' rotation.

    #[test]
    fn yaw_facing_inverts_avatar_front_dir() {
        for (dx, dz) in [(1.0f32, 0.0f32), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0), (0.6, 0.8), (-0.5, -0.5)] {
            let phi = yaw_facing(dx, dz);
            let (fx, fz) = avatar_front_dir(phi);
            let n = (dx * dx + dz * dz).sqrt();
            assert!(
                (fx - dx / n).abs() < 1e-5 && (fz - dz / n).abs() < 1e-5,
                "avatar with yaw_facing({dx},{dz}) must face ({dx},{dz}); got ({fx},{fz})"
            );
        }
    }

    #[test]
    fn avatar_front_dir_matches_the_builder_convention_at_zero() {
        // At model yaw 0 the front (-Z face) points toward +X (per the
        // ps.yaw+π/2 rotation in build_skin_part_vertices). Pins the convention
        // so a builder change can't silently desync the facing math.
        let (fx, fz) = avatar_front_dir(0.0);
        assert!((fx - 1.0).abs() < 1e-5 && fz.abs() < 1e-5, "yaw 0 faces +X; got ({fx},{fz})");
    }

    #[test]
    fn jump_pose_differs_from_idle() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let mut idle = avatar_state(item_kind::EMPTY, 0, 0);
        idle.anim_state = 0; // idle
        let mut jumping = avatar_state(item_kind::EMPTY, 0, 0);
        jumping.anim_state = 2; // airborne
        // Same time_ticks so any difference is the pose, not the walk clock.
        // Compare the skin meshes (.0); the bare-handed avatars have no held mesh.
        let (idle_v, _) = build_player_avatar_vertices(&idle, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        let (jump_v, _) = build_player_avatar_vertices(&jumping, 0.0, 10, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(
            positions_differ(&idle_v, &jump_v),
            "airborne (anim_state==2) pose must visibly differ from idle (anim_state==0)"
        );
    }

    #[test]
    fn arm_override_replaces_walk_swing() {
        let arm = test_part(false);
        // walk_swing alone vs arm_override should produce the override's rotation,
        // not the walk_swing's — so the two must differ when they disagree.
        let walking = part_verts(&arm, PartPose { walk_swing: 0.3, ..PartPose::default() });
        let overridden = part_verts(
            &arm,
            PartPose { walk_swing: 0.3, arm_override: Some(swing_angle(0.5)), ..PartPose::default() },
        );
        assert!(positions_differ(&walking, &overridden));
        // And the override result equals using that angle as the sole walk_swing.
        let as_swing = part_verts(&arm, PartPose { walk_swing: swing_angle(0.5), ..PartPose::default() });
        assert!(!positions_differ(&overridden, &as_swing));
    }

    #[test]
    fn build_skin_part_local_arm_count_and_inflate() {
        // The right arm with its base-face rect must produce a full cuboid =
        // 6 faces × 6 verts = 36 (one box).
        let arm = &player_model()[3]; // RIGHT_ARM_INDEX
        let base = crate::skin_uv::base_faces(crate::skin_uv::ArmModel::Classic);
        let pose = PartPose { walk_swing: 0.0, head_pitch: 0.0, arm_override: Some(0.0), ..Default::default() };
        let flat = build_skin_part_local(arm, &base[3], 0.0, pose, 0);
        assert_eq!(flat.len(), 36, "one box = 6 faces × 6 verts");

        // A non-zero inflate must push every corner outward: each |coord| on the
        // inflated box is >= its zero-inflate counterpart (same winding/order),
        // and strictly greater for at least one vertex.
        let puffed = build_skin_part_local(arm, &base[3], 0.05, pose, 0);
        assert_eq!(puffed.len(), flat.len(), "inflate must not change vertex count");
        let mut any_moved_out = false;
        for (f, p) in flat.iter().zip(&puffed) {
            for axis in 0..3 {
                assert!(
                    p.position[axis].abs() + 1e-6 >= f.position[axis].abs(),
                    "inflate must not pull any corner inward on axis {axis}",
                );
                if p.position[axis].abs() > f.position[axis].abs() + 1e-6 {
                    any_moved_out = true;
                }
            }
        }
        assert!(any_moved_out, "a non-zero inflate must move corners outward");
    }

    #[test]
    fn avatar_skin_verts_carry_the_given_layer() {
        use crate::protocol::item_kind;
        let reg = crate::block::BlockRegistry::new();
        let ps = avatar_state(item_kind::EMPTY, 0, 0);
        // Layer 3: every skin vert must carry tex_layer == 3.
        let (skin, _held) = build_player_avatar_vertices(&ps, 0.0, 0, 3, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(!skin.is_empty());
        assert!(
            skin.iter().all(|v| v.tex_layer == 3),
            "all skin verts must carry the player's layer (3)"
        );
        // Layer 0 still works (Phase 2 / default behaviour).
        let (skin0, _) = build_player_avatar_vertices(&ps, 0.0, 0, 0, 1.0, &reg, crate::skin_uv::ArmModel::Classic);
        assert!(
            skin0.iter().all(|v| v.tex_layer == 0),
            "layer 0 still works (Phase 2)"
        );
    }

    // --- species tint (mob-species-tint foundation, 2026-07-11) ----------

    /// The (reuse species, donor) pairs of MODEL_CACHE. The four villager-tier
    /// humans are deliberately absent (bespoke mesh pending; layer budget).
    const TINTED_REUSE: &[(MobType, MobType)] = &[
        (MobType::Fox, MobType::Wolf),
        (MobType::Cat, MobType::Wolf),
        (MobType::Crab, MobType::Rabbit),
        (MobType::PolarBear, MobType::Bear),
        (MobType::Reindeer, MobType::Horse),
        (MobType::Donkey, MobType::Horse),
        (MobType::Mule, MobType::Horse),
        (MobType::Parrot, MobType::Chicken),
        (MobType::GlowSquid, MobType::Squid),
    ];

    #[test]
    fn mesh_reuse_species_render_their_own_tinted_coats() {
        // Species identity comes from the tinted run, not the donor coat —
        // a Fox must share NO texture layer with the Wolf model it borrows.
        for &(reuse, donor) in TINTED_REUSE {
            let donor_faces: std::collections::HashSet<u32> =
                mob_model(donor).iter().flat_map(|p| p.tex_faces).collect();
            for part in mob_model(reuse) {
                for f in part.tex_faces {
                    assert!(
                        !donor_faces.contains(&f),
                        "{reuse:?} still wears its {donor:?} donor coat (layer {f})"
                    );
                }
            }
        }
    }

    #[test]
    fn donor_models_keep_their_original_untinted_layers() {
        for donor in [
            MobType::Wolf, MobType::Rabbit, MobType::Bear,
            MobType::Horse, MobType::Chicken, MobType::Squid,
        ] {
            for part in mob_model(donor) {
                for f in part.tex_faces {
                    assert!(
                        f < crate::texture_gen::PRE_TINT_LAYER_COUNT,
                        "{donor:?} must be untouched by the tint remap (layer {f})"
                    );
                }
            }
        }
    }

    // ── #19 shell attach + Phase C squash & stretch ──────────────────────────

    /// A micro-model of `n` DISJOINT single-voxel cubes (spaced 2 apart on X, so
    /// nothing merges). Each cube bakes to 6 quads = 24 verts / 36 indices, so
    /// `n >= 2` gives a shell whose emitted vertex count is provably NOT the 36
    /// of a cuboid part.
    fn disjoint_cubes_micro(n: u8) -> crate::micro_model::MicroModelData {
        crate::micro_model::MicroModelData {
            version: crate::micro_model::MICRO_MODEL_VERSION,
            scale: crate::micro_model::MICRO_SCALE_8,
            voxels: (0..n)
                .map(|i| crate::micro_model::MicroVoxel {
                    mx: i * 2,
                    my: 0,
                    mz: 0,
                    block_id: crate::block::STONE,
                })
                .collect(),
            author_npub: String::new(),
            derivation_chain: Vec::new(),
        }
    }

    fn rig_verts(
        rig: &crate::skeleton::RiggedModel,
        micro: Option<&crate::micro_model_registry::MicroModelRegistry>,
        clip: crate::anim_set::AnimClip,
        t: f32,
    ) -> Vec<Vertex> {
        let blocks = crate::block::BlockRegistry::new();
        let mut v = Vec::new();
        build_rigged_vertices(&mut v, rig, &blocks, micro, Vec3::ZERO, 0.0, clip, t, (1.0, 1.0));
        v
    }

    #[test]
    fn rigged_part_with_a_micro_model_emits_the_baked_shell_not_a_cuboid() {
        let blocks = crate::block::BlockRegistry::new();
        let mut micro = crate::micro_model_registry::MicroModelRegistry::new();
        micro.register(crate::block::STONE, disjoint_cubes_micro(2), &blocks);
        let baked = micro.get(crate::block::STONE).expect("registered").mesh.clone();
        // Two disjoint cubes → 12 quads: 48 baked verts / 72 indices. The entity
        // pipeline is a plain triangle list, so the shell emits one vertex per
        // index — decisively NOT the 36 of a cuboid part.
        assert_eq!(baked.vertices.len(), 48);
        assert_eq!(baked.indices.len(), 72);

        let mut rig = crate::skeleton::RiggedModel::new(crate::skeleton::SkeletonKind::Biped, "s");
        rig.attach("head", crate::block::STONE);

        let shell = rig_verts(&rig, Some(&micro), crate::anim_set::AnimClip::Walk, 0.0);
        assert_eq!(shell.len(), baked.indices.len(), "shell = one vertex per baked index");
        assert_ne!(shell.len(), 36, "a shell part is not a cuboid");

        // Same rig, no registry to consult ⇒ the original cuboid path, unchanged.
        let cuboid = rig_verts(&rig, None, crate::anim_set::AnimClip::Walk, 0.0);
        assert_eq!(cuboid.len(), 36, "an unregistered block still draws a 36-vert cuboid");

        // And a rig mixing both gets both: cuboid part + shell part.
        let mut mixed = crate::skeleton::RiggedModel::new(crate::skeleton::SkeletonKind::Biped, "m");
        mixed.attach("head", crate::block::STONE); // shell
        mixed.attach("body", crate::block::OAK_PLANKS); // cuboid (unregistered)
        let both = rig_verts(&mixed, Some(&micro), crate::anim_set::AnimClip::Walk, 0.0);
        assert_eq!(both.len(), baked.indices.len() + 36);
    }

    #[test]
    fn shell_is_fitted_into_the_part_box_and_centred_on_its_origin() {
        let blocks = crate::block::BlockRegistry::new();
        let mut micro = crate::micro_model_registry::MicroModelRegistry::new();
        micro.register(crate::block::STONE, disjoint_cubes_micro(2), &blocks);

        // `body` is not animated, so at t=0 with the Walk clip there is no
        // rotation: the emitted vertices are the fitted shell in model space.
        let mut rig = crate::skeleton::RiggedModel::new(crate::skeleton::SkeletonKind::Biped, "s");
        rig.attach("body", crate::block::STONE);
        let v = rig_verts(&rig, Some(&micro), crate::anim_set::AnimClip::Walk, 0.0);

        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for x in &v {
            let p = Vec3::from(x.position);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        let body = &crate::skeleton::SkeletonKind::Biped.parts()[1];
        let centre = (lo + hi) * 0.5;
        assert!((centre - body.origin).length() < 1e-4, "shell centres on the part origin");
        // It fits inside the part's size box on every axis, and touches it on at
        // least one (uniform fit ⇒ the tightest axis is exactly filled).
        let extent = hi - lo;
        for a in 0..3 {
            assert!(extent[a] <= body.size[a] + 1e-4, "axis {a} fits the part box");
        }
        assert!(
            (0..3).any(|a| (extent[a] - body.size[a]).abs() < 1e-4),
            "the tightest axis fills the box exactly (aspect preserved)"
        );
    }

    #[test]
    fn posed_shell_rotates_about_the_part_pivot() {
        let blocks = crate::block::BlockRegistry::new();
        let mut micro = crate::micro_model_registry::MicroModelRegistry::new();
        micro.register(crate::block::STONE, disjoint_cubes_micro(2), &blocks);

        // `arm_l` is animated, so two walk phases give two different X rotations.
        let mut rig = crate::skeleton::RiggedModel::new(crate::skeleton::SkeletonKind::Biped, "s");
        rig.attach("arm_l", crate::block::STONE);
        let a = rig_verts(&rig, Some(&micro), crate::anim_set::AnimClip::Walk, 0.0);
        let b = rig_verts(&rig, Some(&micro), crate::anim_set::AnimClip::Walk, 0.6);
        assert_eq!(a.len(), b.len());
        assert!(positions_differ(&a, &b), "the shell actually poses with the gait");

        // Rendered at yaw 0 / origin, so model space == world space: a rotation
        // about the pivot preserves every vertex's distance from that pivot.
        let pivot = crate::skeleton::SkeletonKind::Biped.parts()[2].pivot;
        for (x, y) in a.iter().zip(&b) {
            let da = (Vec3::from(x.position) - pivot).length();
            let db = (Vec3::from(y.position) - pivot).length();
            assert!((da - db).abs() < 1e-4, "rotation is about the pivot, {da} vs {db}");
        }
    }

    #[test]
    fn part_pose_scale_squashes_about_the_pivot() {
        // Phase C rung 2: scale [1, 0.5, 1] halves the part's Y extent, and does
        // it about the pivot (the pivot's own Y is a fixed point).
        let arm = &player_model()[2]; // arm_l
        let plain = part_verts(arm, PartPose::default());
        let squashed = part_verts(arm, PartPose { scale: [1.0, 0.5, 1.0], ..PartPose::default() });

        let y_extent = |v: &[Vertex]| {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for x in v {
                lo = lo.min(x.position[1]);
                hi = hi.max(x.position[1]);
            }
            hi - lo
        };
        assert!((y_extent(&squashed) - y_extent(&plain) * 0.5).abs() < 1e-5, "Y halves");

        // X/Z untouched, and the offsets from the pivot really did halve.
        for (p, q) in plain.iter().zip(&squashed) {
            assert!((p.position[0] - q.position[0]).abs() < 1e-6);
            assert!((p.position[2] - q.position[2]).abs() < 1e-6);
            let want = arm.pivot.y + (p.position[1] - arm.pivot.y) * 0.5;
            assert!((q.position[1] - want).abs() < 1e-5);
        }
    }

    #[test]
    fn default_pose_scale_leaves_every_mob_model_byte_identical() {
        // The Phase C regression guard: `PartPose::default()` must carry an
        // identity scale AND the transform must skip it entirely, so three
        // representative species emit exactly the pre-Phase-C cuboid corners
        // (centre ± half, yawed, translated — no scale step anywhere).
        assert_eq!(PartPose::default().scale, [1.0, 1.0, 1.0]);
        let pos = Vec3::new(3.0, 64.0, -7.0);
        let yaw = 0.9_f32;
        for kind in [MobType::Cow, MobType::Chicken, MobType::Pig] {
            for part in mob_model(kind) {
                let mut got = Vec::new();
                build_part_vertices(&mut got, part, pos, yaw, PartPose::default(), None, 1.0);
                assert_eq!(got.len(), 36);
                // Independent recomputation of the pre-Phase-C maths.
                let half = part.size * 0.5;
                let c = part.origin;
                let (cy, sy) = (yaw.cos(), yaw.sin());
                let mut want: Vec<Vec3> = Vec::new();
                for &(sx, sv, sz) in &[
                    (-1.0, -1.0, -1.0), (1.0, -1.0, -1.0), (1.0, 1.0, -1.0), (-1.0, 1.0, -1.0),
                    (-1.0, -1.0, 1.0), (1.0, -1.0, 1.0), (1.0, 1.0, 1.0), (-1.0, 1.0, 1.0),
                ] {
                    let p = Vec3::new(c.x + sx * half.x, c.y + sv * half.y, c.z + sz * half.z);
                    want.push(Vec3::new(
                        p.x * cy - p.z * sy + pos.x,
                        p.y + pos.y,
                        p.x * sy + p.z * cy + pos.z,
                    ));
                }
                // Every emitted vertex is one of the 8 recomputed corners.
                for v in &got {
                    let p = Vec3::from(v.position);
                    assert!(
                        want.iter().any(|w| (*w - p).length() < 1e-5),
                        "{kind:?} emitted a vertex the pre-Phase-C maths never produced"
                    );
                }
            }
        }
    }

    #[test]
    fn bounce_clip_squashes_a_rig_without_moving_its_pivot() {
        let blocks = crate::block::BlockRegistry::new();
        let mut rig = crate::skeleton::RiggedModel::new(crate::skeleton::SkeletonKind::Biped, "b");
        rig.attach("body", crate::block::STONE);
        let mut flat = Vec::new();
        // t = 0 ⇒ sin(0) = 0 ⇒ identity scale ⇒ the plain bind pose.
        build_rigged_vertices(
            &mut flat, &rig, &blocks, None, Vec3::ZERO, 0.0,
            crate::anim_set::AnimClip::Bounce, 0.0, (1.0, 1.0),
        );
        let mut squashed = Vec::new();
        // A quarter-period in, the squash is at its peak.
        build_rigged_vertices(
            &mut squashed, &rig, &blocks, None, Vec3::ZERO, 0.0,
            crate::anim_set::AnimClip::Bounce, std::f32::consts::FRAC_PI_2 / 2.2,
            (1.0, 1.0),
        );
        let extent = |v: &[Vertex], a: usize| {
            let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for x in v {
                lo = lo.min(x.position[a]);
                hi = hi.max(x.position[a]);
            }
            hi - lo
        };
        assert!(extent(&squashed, 1) < extent(&flat, 1), "bounce squashes on Y");
        assert!(extent(&squashed, 0) > extent(&flat, 0), "and widens on X");
    }
}
