//! Small in-memory worlds for the bloom tests.

use glam::{IVec3, Vec3};

use super::field::{BloomField, TickInput, TickOutput};
use super::RegionKey;
use crate::biome::{BiomeGenerator, Generator};
use crate::block::{self, BlockId, BlockRegistry};
use crate::world::World;

/// A world, its generator, an (empty) ECS and a bloom field, ticked together.
pub struct Harness {
    pub world: World,
    pub bg: BiomeGenerator,
    pub ecs: hecs::World,
    pub registry: BlockRegistry,
    pub field: BloomField,
    pub local: Vec<Vec3>,
    pub remote: Vec<Vec3>,
    pub creative: bool,
    pub allow_grey: bool,
}

impl Harness {
    /// A flat-less empty world (no generation) with one player at the origin.
    pub fn blank() -> Self {
        Self::with_generator(1, Generator::V3)
    }

    pub fn with_generator(seed: u32, generator: Generator) -> Self {
        let mut world = World::new();
        world.generator = generator;
        Self {
            world,
            bg: BiomeGenerator::new(seed, generator),
            ecs: hecs::World::new(),
            registry: BlockRegistry::new(),
            field: BloomField::new(),
            local: vec![Vec3::new(8.0, 90.0, 8.0)],
            remote: Vec::new(),
            creative: false,
            allow_grey: true,
        }
    }

    /// Generate the chunk column of region `key` with the world generator.
    pub fn generate(&mut self, key: RegionKey) {
        self.world.generate_column(key.0, key.1, &self.bg);
    }

    pub fn tick(&mut self) -> TickOutput {
        self.field.tick(&TickInput {
            world: &self.world,
            ecs: &self.ecs,
            biome_gen: &self.bg,
            registry: &self.registry,
            local_players: &self.local,
            remote_players: &self.remote,
            creative: self.creative,
            dt: 0.05,
            allow_grey_event: self.allow_grey,
        })
    }

    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    /// Tick until region `key` has been scanned (a few ticks at most).
    pub fn settle(&mut self, key: RegionKey) {
        for _ in 0..400 {
            if self.field.region_terms(key).is_some() {
                return;
            }
            self.tick();
        }
        panic!("region {key:?} was never scanned");
    }

    /// Re-scan after an edit: forget the field and settle again.
    pub fn rescan(&mut self, key: RegionKey) {
        self.field = BloomField::new();
        self.settle(key);
    }

    /// Stand the local player in the middle of region `key`.
    pub fn at(&mut self, key: RegionKey) {
        self.local = vec![Vec3::new(key.0 as f32 * 16.0 + 8.0, 90.0, key.1 as f32 * 16.0 + 8.0)];
    }

    /// The life term of `key` after a fresh scan and one wildlife census.
    pub fn eco(&mut self, key: RegionKey) -> f32 {
        self.rescan(key);
        self.run(22);
        self.field.region_terms(key).unwrap().0
    }

    /// Put an animal of `kind` in region `key`.
    pub fn animal(&mut self, key: RegionKey, kind: crate::mob::MobType) {
        let at = Vec3::new(key.0 as f32 * 16.0 + 4.0, 70.0, key.1 as f32 * 16.0 + 4.0);
        self.ecs.spawn((crate::entity::Position(at), crate::entity::MobKind(kind)));
    }

    /// Strip region `key` to a bare stone quarry floor: everything above
    /// `floor_y` goes, the floor is stone (not placed by a player).
    pub fn strip(&mut self, key: RegionKey, floor_y: i32) {
        for dz in 0..16 {
            for dx in 0..16 {
                let (x, z) = (key.0 * 16 + dx, key.1 * 16 + dz);
                for y in (floor_y + 1)..=255 {
                    if self.world.get_block(x, y, z) != block::AIR {
                        self.world.set_block(x, y, z, block::AIR);
                    }
                }
                self.world.set_block(x, floor_y, z, block::STONE);
            }
        }
    }

    pub fn probe_eco(&mut self, min: IVec3, max: IVec3) -> f32 {
        self.field = BloomField::new();
        self.field.add_probe(7, min, max);
        for _ in 0..400 {
            self.tick();
            if self.field.probe(7).is_some_and(|p| p.scanned) {
                break;
            }
        }
        self.field.probe(7).unwrap().eco
    }

    /// Put `id` on the columns `cols` (`(dx, dz)` in the region) at `y`, as a
    /// player plants it.
    pub fn plant(&mut self, key: RegionKey, cols: impl IntoIterator<Item = (i32, i32)>, y: i32, id: BlockId) {
        for (dx, dz) in cols {
            self.world.place_player_block(key.0 * 16 + dx, y, key.1 * 16 + dz, id);
        }
    }
}

/// `n` distinct columns of a 16 x 16 region, row by row.
pub fn first_columns(n: usize) -> Vec<(i32, i32)> {
    (0..n as i32).map(|i| (i % 16, i / 16)).collect()
}
