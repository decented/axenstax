//! Runtime dialogue identity and the clock of the existing personal holder.
//! Neither a view nor a clock owns a second copy of progression.
use crate::progression::{GameplayId, GameplayRecord};

#[derive(Clone, Debug)]
pub(crate) struct QuestSession {
    pub entity: hecs::Entity,
    pub target: u32,
    pub npc: GameplayId,
    pub incarnation: u64,
    pub actor_respawns: u32,
    pub offer: Option<crate::quest_play::Offer>,
    pub revision: u64,
    pub window_digest: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GameplayClock {
    anchor: Option<u64>,
    now: u64,
}
impl GameplayClock {
    /// Restoring starts this gameplay clock now. Time outside play is excluded.
    pub(crate) fn restore(&mut self, tick: u64) {
        self.anchor = Some(tick);
        self.now = tick;
    }
    pub(crate) fn project(&self, record: &GameplayRecord, tick: u64) -> (GameplayRecord, bool) {
        crate::quest_play::project_timers(
            record,
            self.anchor.map_or(0, |anchor| tick.saturating_sub(anchor)),
        )
    }
    pub(crate) fn observe(&mut self, tick: u64) {
        if self.anchor.is_none() {
            self.restore(tick);
        } else {
            self.now = tick;
        }
    }
    pub(crate) fn captured(&self, record: &GameplayRecord) -> GameplayRecord {
        self.project(record, self.now).0
    }
    /// Commit only with the projected SAME holder, after admission succeeds.
    pub(crate) fn committed(&mut self, tick: u64) {
        self.restore(tick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn d2c3_gameplay_clock_excludes_offline_ticks_and_projects_all_timers_together() {
        let context = crate::progression::GameplayContext::Guest {
            session: [1; 16],
            generation: 1,
        };
        let mut record = GameplayRecord::empty(context).unwrap();
        record.quests.declines.push(([2; 16], 100));
        record.quests.penalty_left = 30;
        let mut clock = GameplayClock::default();
        clock.restore(20);
        let (projected, expired) = clock.project(&record, 35);
        assert!(!expired);
        assert_eq!(projected.quests.declines[0].1, 85);
        assert_eq!(projected.quests.penalty_left, 15);
        assert_eq!(
            record.quests.penalty_left, 30,
            "a projection never mutates the holder"
        );
        clock.committed(35);
        let bytes = projected.encode().unwrap();
        let restored = GameplayRecord::decode(&bytes, context).unwrap();
        clock.restore(9000);
        assert_eq!(
            clock.project(&restored, 9000).0,
            restored,
            "offline time does not consume cooldowns"
        );
        let (expired, changed) = clock.project(&restored, 9015);
        assert!(changed);
        assert_eq!(expired.quests.penalty_left, 0);
        assert_eq!(expired.quests.declines[0].1, 70);
        assert_eq!(
            expired.revision, record.revision,
            "only an actual committed expiry advances the revision"
        );
    }
}
