//! The Trials island (§C.3), played: a visit (build the island as its
//! columns load, put the player at the plaza or back at their doorway, hang
//! the postcards), the doorways (walking into one opens the trial's hero
//! card, stepping out closes it, Play launches the trial), and the trip home
//! (a trial launched from the island returns to that doorway; "Back to menu"
//! still goes to the lobby). Layout: `trials_island_layout`; world writes:
//! `trials_island_build`.

use web_time::Instant;

use crate::trials_island_build as build;
use crate::trials_island_layout::{IslandLayout, SPAWN};

/// How long a visit waits for the arrival spot's floor to load before it
/// leaves the player where the save put them.
const ARRIVAL_WAIT_TICKS: u32 = 200;

/// Where a visit puts the player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// The middle of the plaza (a visit from the gallery).
    Plaza,
    /// In front of this trial's doorway (back from the trial).
    Doorway(String),
    /// Already placed.
    Done,
}

/// A trial launched from the island: where to come back to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IslandReturn {
    pub token: String,
    /// The trial's world folder (or its prefix, for a `KeepNew` arena): the
    /// return only fires from that world.
    pub trial_folder: String,
}

/// An open hero card in a doorway.
pub struct IslandCard {
    pub token: &'static str,
    pub card: crate::trials_gallery::Card,
    pub postcard: Option<egui::TextureHandle>,
}

/// One visit to the island, held on `GameState` while the island world is up.
pub struct IslandVisit {
    pub layout: IslandLayout,
    /// Chunk columns not yet written (their floor had not loaded).
    pub pending: Vec<(i32, i32)>,
    pub arrival: Arrival,
    waited: u32,
    /// The doorway whose opening the feet were in last tick.
    inside: Option<&'static str>,
    pub card: Option<IslandCard>,
}

/// What one tick of the doorway watch decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorwayStep {
    Nothing,
    Open(&'static str),
    Close,
}

/// The doorway watch, pure. A card opens on stepping INTO an opening (from
/// outside it), so arriving in one, or standing still in one after closing
/// the card, opens nothing; it closes once the feet leave the doorway's
/// plinth. Returns the step and the opening the feet are now in.
pub fn doorway_step(
    layout: &IslandLayout,
    card: Option<&str>,
    inside_before: Option<&'static str>,
    feet: [f32; 3],
) -> (DoorwayStep, Option<&'static str>) {
    let now = layout.doorway_opening_at(feet).map(|d| d.token);
    if let Some(open) = card {
        let holds = layout.doorway(open).is_some_and(|d| d.holds(feet));
        return (if holds { DoorwayStep::Nothing } else { DoorwayStep::Close }, now);
    }
    match now {
        Some(t) if inside_before != Some(t) => (DoorwayStep::Open(t), now),
        _ => (DoorwayStep::Nothing, now),
    }
}

/// The doorway to come back to, if this world is the trial `ret` launched.
pub fn return_token(ret: Option<IslandReturn>, world_name: &str) -> Option<String> {
    ret.filter(|r| world_name.starts_with(&r.trial_folder)).map(|r| r.token)
}

/// The world folder (or prefix) a launched trial plays in (`PlayScenario`'s rule).
fn trial_folder(def: &crate::scenario::ScenarioDef) -> String {
    crate::scenario::experience_world_slug(&def.display_name)
}

impl crate::GameState {
    /// Is the player on the island?
    pub(crate) fn on_island(&self) -> bool {
        self.island.is_some()
    }

    pub(crate) fn island_card_open(&self) -> bool {
        self.island.as_ref().is_some_and(|v| v.card.is_some())
    }

    /// No breaking or placing in the island's built area (aimed at, or next to it).
    pub(crate) fn island_protects(&self, target: Option<[i32; 3]>) -> bool {
        self.on_island()
            && target.is_some_and(|t| crate::trials_island_layout::is_protected(t[0], t[2]))
    }

    /// One tick on the island: start the visit when the island world is
    /// entered, then build, place, hang and watch the doorways.
    pub(crate) fn tick_trials_island(&mut self) {
        if self.island.is_none() {
            if self.joined() || self.world_name != build::island_folder() || self.players.is_empty() {
                return;
            }
            // The island is a place, not a run: no scenario, no race.
            self.scenario = None;
            self.active_trial = None;
            let layout = build::layout_for(&self.trial_bests);
            let pending = IslandLayout::columns(crate::chunk::CHUNK_SIZE as i32);
            let arrival = match self.island_arrival.take() {
                Some(token) => Arrival::Doorway(token),
                None => Arrival::Plaza,
            };
            log::info!("Trials island: visit starts ({} doorways, arrival {arrival:?})", layout.doorways.len());
            self.island = Some(IslandVisit { layout, pending, arrival, waited: 0, inside: None, card: None });
            self.hang_island_postcards();
        }
        self.island_build_loaded_columns();
        self.island_place_player();
        self.island_watch_doorways();
    }

    fn island_build_loaded_columns(&mut self) {
        let Some(visit) = self.island.as_mut() else { return };
        if visit.pending.is_empty() {
            return;
        }
        let floor = build::floor_cy();
        let (ready, waiting): (Vec<_>, Vec<_>) =
            visit.pending.iter().partition(|(cx, cz)| self.world.has_chunk(*cx, floor, *cz));
        visit.pending = waiting;
        let mut dirty = Vec::new();
        for (cx, cz) in ready {
            let changed = build::apply_column(&mut self.world, &visit.layout, cx, cz);
            if !changed.is_empty() {
                dirty.push((cx, cz));
            }
        }
        for (cx, cz) in dirty {
            crate::lighting::run_initial_pass_for_column(&mut self.world, cx, cz, &self.registry);
            for cy in floor..=floor + 1 {
                for dx in -1..=1 {
                    for dz in -1..=1 {
                        self.mark_chunk_dirty((cx + dx, cy, cz + dz));
                    }
                }
            }
        }
    }

    fn island_place_player(&mut self) {
        let Some(visit) = self.island.as_mut() else { return };
        let (pos, yaw) = match &visit.arrival {
            Arrival::Done => return,
            Arrival::Plaza => (SPAWN, 0.0),
            Arrival::Doorway(token) => match visit.layout.doorway(token) {
                Some(d) => (d.return_pos, d.return_yaw),
                None => (SPAWN, 0.0),
            },
        };
        let cs = crate::chunk::CHUNK_SIZE as i32;
        let (cx, cz) = ((pos[0].floor() as i32).div_euclid(cs), (pos[2].floor() as i32).div_euclid(cs));
        if !self.world.has_chunk(cx, build::floor_cy(), cz) {
            visit.waited += 1;
            if visit.waited > ARRIVAL_WAIT_TICKS {
                log::warn!("Trials island: arrival spot never loaded; leaving the player where they are");
                visit.arrival = Arrival::Done;
            }
            return;
        }
        visit.arrival = Arrival::Done;
        visit.inside = visit.layout.doorway_opening_at(pos).map(|d| d.token);
        let p = &mut self.players[0];
        p.player.pos = glam::Vec3::from(pos);
        p.player.velocity = glam::Vec3::ZERO;
        p.player.reset_fall();
        p.camera.yaw = yaw;
        p.camera.pitch = 0.0;
    }

    fn island_watch_doorways(&mut self) {
        let feet = self.players[0].player.pos.to_array();
        let Some(visit) = self.island.as_mut() else { return };
        if visit.arrival != Arrival::Done {
            return;
        }
        let (step, now) = doorway_step(&visit.layout, visit.card.as_ref().map(|c| c.token), visit.inside, feet);
        visit.inside = now;
        match step {
            DoorwayStep::Nothing => {}
            DoorwayStep::Close => visit.card = None,
            DoorwayStep::Open(token) => self.island_open_card(token),
        }
    }

    /// Open a doorway's hero card (the gallery's card for that trial).
    pub(crate) fn island_open_card(&mut self, token: &'static str) {
        let Some(card) = crate::trials_gallery::card_for(token, &self.trial_bests) else { return };
        let postcard = crate::postcard::newest_png(token)
            .and_then(|png| crate::postcard_flow::texture_from_png(&self.renderer.egui.ctx, "island_postcard", &png));
        if self.trial_bests.mark_card_opened(token) {
            self.trial_bests.save();
        }
        if let Some(visit) = self.island.as_mut() {
            visit.card = Some(IslandCard { token, card, postcard });
        }
        // The card has a button to press (native frees it every frame too,
        // `cursor_must_be_free`; the web has only this).
        self.release_cursor();
    }

    /// Draw an open doorway card; Play (or Enter) launches its trial.
    pub(crate) fn draw_island_card(&mut self) {
        let Some(open) = self.island.as_ref().and_then(|v| v.card.as_ref()) else { return };
        let ctx = self.renderer.egui.ctx.clone();
        let play = crate::trials_island_ui::draw_card(&ctx, &open.card, open.postcard.as_ref().map(|t| t.id()));
        if play {
            let token = open.token;
            self.island_play(token);
        }
    }

    /// Leave the island for this doorway's trial. The player is stood at the
    /// doorway's return spot first, so the island saves them there.
    pub(crate) fn island_play(&mut self, token: &'static str) {
        let Some(visit) = self.island.as_mut() else { return };
        let Some(open) = visit.card.take().filter(|c| c.token == token) else { return };
        let back = visit.layout.doorway(token).map(|d| (d.return_pos, d.return_yaw));
        if self.networked() {
            self.toast = Some((
                crate::world_exit::ARENA_ONLINE_REFUSAL.to_string(),
                Instant::now() + std::time::Duration::from_secs(5),
            ));
            return;
        }
        if let Some((pos, yaw)) = back {
            let p = &mut self.players[0];
            p.player.pos = glam::Vec3::from(pos);
            p.player.velocity = glam::Vec3::ZERO;
            p.camera.yaw = yaw;
        }
        let def = open.card.def.clone();
        let ret = IslandReturn { token: token.to_string(), trial_folder: trial_folder(&def) };
        if !self.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby) {
            return;
        }
        log::info!("Trials island: '{token}' launched from its doorway");
        self.pending_menu_action =
            Some((crate::menu::MenuAction::PlayScenario { def: Box::new(def) }, Instant::now()));
        self.island_return = Some(ret);
        self.release_cursor();
    }

    /// A trial's end card "Back to Trials": to the island's doorway when the
    /// trial was launched from there, else to the lobby as before.
    pub(crate) fn back_to_trials(&mut self) {
        let ret = return_token(self.island_return.take(), &self.world_name);
        if !self.leave_world(crate::world_exit::SaveChoice::Save, crate::world_exit::ExitTo::Lobby) {
            return;
        }
        if let Some(token) = ret {
            log::info!("Trials island: back to the '{token}' doorway");
            self.pending_menu_action = Some((
                crate::menu::MenuAction::PlayScenario { def: Box::new(crate::scenario::trials_island_def()) },
                Instant::now(),
            ));
            self.island_arrival = Some(token);
        }
    }

    /// Hang each open doorway's newest postcard on its arch (a wall exhibit,
    /// the image written into the island world's exhibit folder; on the web,
    /// handed to the exhibit loader in memory).
    fn hang_island_postcards(&mut self) {
        let Some(visit) = self.island.as_ref() else { return };
        let pngs: Vec<(&'static str, Vec<u8>)> = visit
            .layout
            .doorways
            .iter()
            .filter_map(|d| crate::postcard::newest_png(d.token).map(|png| (d.token, png)))
            .collect();
        let wanted = build::exhibits(&visit.layout, |t| pngs.iter().any(|(p, _)| *p == t));
        let mut fresh_image = false;
        for (token, png) in &pngs {
            let image_ref = build::exhibit_ref(token);
            #[cfg(not(target_arch = "wasm32"))]
            {
                let path = crate::save::exhibit_image_path(&self.world_name, &image_ref);
                if std::fs::read(&path).ok().as_deref() != Some(png.as_slice()) {
                    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, png));
                    match written {
                        Ok(()) => fresh_image = true,
                        Err(e) => log::warn!("Trials island: postcard exhibit {}: {e}", path.display()),
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                self.pending_exhibit_images.retain(|i| i.image_ref != image_ref);
                self.pending_exhibit_images
                    .push(crate::world_archive::ExhibitImage { image_ref, bytes: png.clone() });
                fresh_image = true;
            }
        }
        let changed = build::merge_exhibits(&mut self.world.exhibits, wanted.into_iter().map(|(_, e)| e).collect());
        if changed || fresh_image {
            self.renderer.clear_paintings();
            self.exhibit_art_requested = false;
            self.exhibit_tex_aspect.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trials::TrialBests;

    fn island() -> IslandLayout {
        build::layout_for(&TrialBests::default())
    }

    fn opening(l: &IslandLayout, token: &str) -> [f32; 3] {
        let d = l.doorway(token).unwrap();
        [d.open_min[0] as f32 + 0.5, d.return_pos[1], d.open_min[1] as f32 + 0.5]
    }

    #[test]
    fn walking_in_opens_the_card_and_stepping_out_closes_it() {
        let l = island();
        let door = opening(&l, "mine");
        let outside = l.doorway("mine").unwrap().return_pos;
        let (step, inside) = doorway_step(&l, None, None, outside);
        assert_eq!((step, inside), (DoorwayStep::Nothing, None));
        let (step, inside) = doorway_step(&l, None, inside, door);
        assert_eq!(step, DoorwayStep::Open("mine"));
        // While open, standing in the doorway does nothing; leaving closes it.
        assert_eq!(doorway_step(&l, Some("mine"), inside, door).0, DoorwayStep::Nothing);
        assert_eq!(doorway_step(&l, Some("mine"), inside, outside).0, DoorwayStep::Close);
    }

    #[test]
    fn arriving_in_a_doorway_or_lingering_after_closing_opens_nothing() {
        let l = island();
        let door = opening(&l, "craft");
        // Already inside last tick (arrived there, or closed the card there).
        assert_eq!(doorway_step(&l, None, Some("craft"), door).0, DoorwayStep::Nothing);
    }

    #[test]
    fn coming_back_lands_only_from_the_trial_that_left() {
        let ret = || Some(IslandReturn { token: "mine".into(), trial_folder: "exp-mine-a-ore".into() });
        assert_eq!(return_token(ret(), "exp-mine-a-ore"), Some("mine".to_string()));
        assert_eq!(return_token(ret(), "my-own-world"), None, "another world goes to the lobby");
        assert_eq!(return_token(None, "exp-mine-a-ore"), None);
    }

    #[test]
    fn the_island_def_is_a_flat_day_locked_resumed_world_with_no_mobs() {
        let d = crate::scenario::trials_island_def();
        assert_eq!(d.objective, crate::scenario::Objective::FreeRoam);
        assert_eq!(d.arena_mode, crate::scenario::ArenaMode::Resume);
        assert_eq!(d.world_type.as_deref(), Some("flat"));
        assert_eq!(d.time_lock.as_deref(), Some("day"));
        assert_eq!(d.mobs_enabled, Some(false));
        assert_eq!(build::island_folder(), "exp-trials-island");
        // A place, not a trial: in no trial list.
        assert!(crate::trials::TRIAL_ORDER.iter().all(|(_, r)| !matches!(r, crate::trials::TrialRef::Challenge("trials-island"))));
        assert!(crate::scenario::challenge_listing().iter().all(|(t, _)| *t != "trials-island"));
    }
}
