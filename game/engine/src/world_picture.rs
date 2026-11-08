//! A world's picture on its lobby card and the home page's Continue hero (home
//! page overhaul spec §6). For now every world shows a placeholder: a small
//! isometric diorama drawn with the Trials diorama code, chosen by the world's
//! mode and type, on that mode's colour band. It is never an empty grey box.
//!
//! Follow-up (not built): capture the world view on save-and-quit to
//! `<world>/picture.png` and paint that here instead.

use std::sync::LazyLock;

use crate::save::WorldMeta;
use crate::trials_diorama::{Cube, DioramaBlock};

/// Which little scene a world's placeholder shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scene {
    /// Grass and an oak: a survival (or adventure) world.
    Meadow,
    /// Iron and glass under a diamond block: a creative world (the game has no quartz block).
    Gallery,
    /// Sand and sandstone: a desert seed, or a blank canvas on sand.
    Desert,
    /// Snow and stone: a blank canvas on snow.
    Snowfield,
    /// Water lapping sand: a blank canvas on water.
    Shore,
    /// Stone and cobble: a blank canvas on stone.
    Quarry,
    /// One glass block in the air: a blank canvas with no ground, or a spectator world.
    Void,
}

impl Scene {
    const ALL: [Scene; 7] = [
        Scene::Meadow,
        Scene::Gallery,
        Scene::Desert,
        Scene::Snowfield,
        Scene::Shore,
        Scene::Quarry,
        Scene::Void,
    ];

    /// The scene's blocks (`/give` names on the diorama grid).
    fn blocks(self) -> &'static [(&'static str, [i32; 3])] {
        match self {
            Scene::Meadow => &[
                ("grass", [-1, 0, 0]),
                ("grass", [0, 0, 0]),
                ("grass", [1, 0, 0]),
                ("grass", [0, 0, 1]),
                ("grass", [0, 0, -1]),
                ("oak_log", [0, 1, 0]),
                ("oak_leaves", [0, 2, 0]),
                ("dirt", [1, 0, 1]),
            ],
            Scene::Gallery => &[
                ("iron_block", [-1, 0, 0]),
                ("iron_block", [0, 0, 0]),
                ("iron_block", [1, 0, 0]),
                ("iron_block", [0, 0, 1]),
                ("glass", [0, 1, 0]),
                ("glass", [1, 1, 0]),
                ("diamond_block", [0, 2, 0]),
            ],
            Scene::Desert => &[
                ("sand", [-1, 0, 0]),
                ("sand", [0, 0, 0]),
                ("sand", [1, 0, 0]),
                ("sand", [0, 0, 1]),
                ("sandstone", [1, 0, 1]),
                ("sandstone", [0, 1, 0]),
                ("sand", [-1, 1, 0]),
            ],
            Scene::Snowfield => &[
                ("snow", [-1, 0, 0]),
                ("snow", [0, 0, 0]),
                ("snow", [1, 0, 0]),
                ("snow", [0, 0, 1]),
                ("stone", [0, 1, 0]),
                ("snow", [0, 2, 0]),
            ],
            Scene::Shore => &[
                ("water", [-1, 0, 0]),
                ("water", [0, 0, 0]),
                ("water", [0, 0, 1]),
                ("sand", [1, 0, 0]),
                ("sand", [1, 0, 1]),
                ("sand", [1, 1, 0]),
            ],
            Scene::Quarry => &[
                ("stone", [-1, 0, 0]),
                ("stone", [0, 0, 0]),
                ("cobblestone", [1, 0, 0]),
                ("stone", [0, 0, 1]),
                ("cobblestone", [0, 1, 0]),
            ],
            Scene::Void => &[("glass", [0, 0, 0])],
        }
    }
}

/// The colour band (and backdrop tint) of a world's mode.
pub fn mode_colour(mode: &str) -> egui::Color32 {
    match mode {
        "creative" => egui::Color32::from_rgb(64, 132, 196),
        "adventure" => egui::Color32::from_rgb(196, 144, 52),
        "spectator" => egui::Color32::from_rgb(140, 104, 196),
        _ => egui::Color32::from_rgb(84, 156, 64),
    }
}

/// The mode's name as a player reads it (never SURVIVAL for an adventure world).
pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "creative" => "Creative",
        "adventure" => "Adventure",
        "spectator" => "Spectator",
        _ => "Survival",
    }
}

/// Which scene a world shows: `desert` says whether the seed's spawn is desert
/// (asked only for a normal world; see [`scene_for`]).
pub fn choose(mode: &str, world_type: &str, ground: &str, desert: impl FnOnce() -> bool) -> Scene {
    if world_type == "flat" {
        return match ground {
            "sand" => Scene::Desert,
            "snow" => Scene::Snowfield,
            "water" => Scene::Shore,
            "stone" => Scene::Quarry,
            "none" => Scene::Void,
            _ if mode == "creative" => Scene::Gallery,
            _ => Scene::Meadow,
        };
    }
    match mode {
        "creative" => Scene::Gallery,
        "spectator" => Scene::Void,
        _ if desert() => Scene::Desert,
        _ => Scene::Meadow,
    }
}

thread_local! {
    /// Whether a (seed, generator) spawns in desert, worked out once per world.
    static DESERT: std::cell::RefCell<std::collections::HashMap<(u32, u32), bool>> = Default::default();
}

/// Whether the world's centre column is desert (cached: building the
/// generator's noise is not a per-frame cost).
fn seed_is_desert(seed: u32, worldgen_version: u32) -> bool {
    DESERT.with(|c| {
        *c.borrow_mut().entry((seed, worldgen_version)).or_insert_with(|| {
            crate::biome::Generator::from_version(worldgen_version)
                .map(|g| crate::biome::BiomeGenerator::new(seed, g).biome_at(0, 0) == crate::biome::Biome::Desert)
                .unwrap_or(false)
        })
    })
}

/// The scene for a world.
pub fn scene_for(meta: &WorldMeta) -> Scene {
    choose(&meta.game_mode, &meta.world_type, &meta.ground, || seed_is_desert(meta.seed, meta.worldgen_version))
}

/// Every scene's resolved blocks, back to front (resolved once).
static SCENES: LazyLock<Vec<(Scene, Vec<Cube>)>> = LazyLock::new(|| {
    Scene::ALL
        .iter()
        .map(|&s| {
            let blocks: Vec<DioramaBlock> =
                s.blocks().iter().map(|(b, at)| DioramaBlock { block: (*b).to_string(), at: *at }).collect();
            (s, crate::trials_gallery::resolve_diorama(&blocks))
        })
        .collect()
});

fn cubes(scene: Scene) -> &'static [Cube] {
    SCENES.iter().find(|(s, _)| *s == scene).map(|(_, c)| c.as_slice()).unwrap_or(&[])
}

/// Paint a world's picture into `rect` (rounded by `radii`, nw/ne/se/sw).
pub fn paint(painter: &egui::Painter, rect: egui::Rect, radii: [f32; 4], meta: &WorldMeta, icons: &[egui::TextureId]) {
    paint_scene(painter, rect, radii, scene_for(meta), &meta.game_mode, icons);
}

/// Paint one scene: the mode's colour as a gradient, the scene in the middle,
/// and a band of the mode's colour along the bottom edge.
pub fn paint_scene(
    painter: &egui::Painter,
    rect: egui::Rect,
    radii: [f32; 4],
    scene: Scene,
    mode: &str,
    icons: &[egui::TextureId],
) {
    let band = mode_colour(mode);
    let top = crate::trials_diorama_ui::lerp_color(crate::brand::SURFACE, band, 0.5);
    let bottom = crate::trials_diorama_ui::lerp_color(crate::brand::SURFACE, band, 0.16);
    crate::trials_diorama_ui::paint_backdrop(painter, rect, radii, top, bottom);
    painter.circle_filled(rect.center(), rect.height() * 0.42, egui::Color32::from_white_alpha(10));
    crate::trials_diorama_ui::paint_diorama(painter, rect.shrink(rect.height() * 0.08), cubes(scene), icons, 0.2);
    let strip = egui::Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - 5.0), rect.max);
    painter.rect_filled(
        strip,
        egui::CornerRadius { nw: 0, ne: 0, se: radii[2] as u8, sw: radii[3] as u8 },
        band,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_resolves_every_block() {
        for s in Scene::ALL {
            assert_eq!(cubes(s).len(), s.blocks().len(), "{s:?}: a block name does not resolve");
        }
    }

    #[test]
    fn the_scene_follows_mode_and_type() {
        let never = || panic!("a non-normal world asked for its biome");
        assert_eq!(choose("survival", "normal", "grass", || false), Scene::Meadow);
        assert_eq!(choose("adventure", "normal", "grass", || false), Scene::Meadow);
        assert_eq!(choose("survival", "normal", "grass", || true), Scene::Desert);
        assert_eq!(choose("creative", "normal", "grass", never), Scene::Gallery);
        assert_eq!(choose("spectator", "normal", "grass", never), Scene::Void);
        assert_eq!(choose("survival", "flat", "sand", never), Scene::Desert);
        assert_eq!(choose("survival", "flat", "snow", never), Scene::Snowfield);
        assert_eq!(choose("survival", "flat", "water", never), Scene::Shore);
        assert_eq!(choose("survival", "flat", "stone", never), Scene::Quarry);
        assert_eq!(choose("creative", "flat", "none", never), Scene::Void);
        assert_eq!(choose("creative", "flat", "grass", never), Scene::Gallery);
        assert_eq!(choose("survival", "flat", "grass", never), Scene::Meadow);
    }

    #[test]
    fn every_mode_has_its_own_name_and_colour() {
        let modes = ["survival", "creative", "adventure", "spectator"];
        let names: std::collections::HashSet<_> = modes.iter().map(|m| mode_label(m)).collect();
        let colours: std::collections::HashSet<_> = modes.iter().map(|m| mode_colour(m).to_array()).collect();
        assert_eq!(names.len(), 4);
        assert_eq!(colours.len(), 4);
        assert_eq!(mode_label("adventure"), "Adventure", "adventure used to read SURVIVAL");
    }
}
