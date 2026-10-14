//! Participant record `participants/<npub>.dat`, schema 1: one player's
//! world-local progression (village reputation, per-mob kill counts, bounty
//! claims) plus an optional owner-only local restore (pose, view, hotbar
//! selection, 36-bit slot-lock mask). Bounded, canonical and strict: sorted
//! unique keys, known mob kinds, scores inside the reputation range, finite
//! pose. Oversized or malformed state refuses — nothing is truncated, clamped
//! or reset to make a transfer pass.
//!
//! The header binds participant, lineage, checkpoint generation and transfer
//! id. The enclosing manifest hashes the whole member and the certificate binds
//! the manifest digest, so the record never embeds the manifest digest itself
//! (that would be circular).
use super::{
    Error, Result,
    codec::{Reader, frame},
};
use crate::mob::MobType;

const DOMAIN: &str = "AxeNStax/handover/participant/v1";
pub const SCHEMA: u32 = 2;
pub const RECORD_LIMIT: usize = 64 * 1024;
pub const REPUTATION_LIMIT: usize = 2048;
pub const KILL_KINDS_LIMIT: usize = 256;
pub const CLAIMS_LIMIT: usize = 4096;
const LOCK_BITS: u64 = (1 << 36) - 1;

/// The context every record of one checkpoint shares.
pub use crate::personal_value::PersonalContext as RecordContext;

/// Owner-only local restore: never applied to anybody but `participant`.
pub use crate::progression::LocalRestore;

#[derive(Clone, Debug, PartialEq)]
pub struct PersonalRecord {
    pub context: RecordContext,
    pub reputation: Vec<((i32, i32), i16)>,
    pub kills: Vec<(MobType, u32)>,
    pub claims: Vec<(u32, u32)>,
    pub local: Option<LocalRestore>,
    pub(crate) revision: u64,
    pub(crate) tracking: crate::progression::KillTracking,
    pub(crate) quests: crate::progression::QuestProgress,
}

fn mob_index(m: MobType) -> Result<u32> {
    let b = bincode::serialize(&m).map_err(|_| Error::Invalid("mob kind"))?;
    let a: [u8; 4] = b.try_into().map_err(|_| Error::Invalid("mob kind"))?;
    Ok(u32::from_le_bytes(a))
}
fn mob_from_index(i: u32) -> Result<MobType> {
    bincode::deserialize(&i.to_le_bytes()).map_err(|_| Error::Invalid("unknown mob kind"))
}

impl PersonalRecord {
    pub(crate) fn from_gameplay(context: RecordContext, value: &crate::progression::GameplayRecord) -> Result<Self> {
        let record = Self { context, reputation: value.reputation.clone(), kills: value.kills.clone(),
            claims: value.claims.clone(), local: value.local, revision: value.revision,
            tracking: value.tracking, quests: value.quests.clone() };
        record.validate()?; Ok(record)
    }
    pub(crate) fn to_gameplay(&self) -> crate::progression::GameplayRecord {
        crate::progression::GameplayRecord { context: crate::progression::GameplayContext::Imported(self.context),
            revision: self.revision, reputation: self.reputation.clone(), kills: self.kills.clone(),
            claims: self.claims.clone(), local: self.local, tracking: self.tracking, quests: self.quests.clone() }
    }
    pub(crate) fn declared_schema(bytes: &[u8]) -> Result<u32> {
        let mut reader = Reader::new(bytes, RECORD_LIMIT)?; reader.label(DOMAIN)?;
        let schema = reader.u32()?;
        if [1, 2].contains(&schema) { Ok(schema) } else { Err(Error::Invalid("participant schema")) }
    }
    #[cfg(test)]
    pub(crate) fn encode_schema1(&self) -> Result<Vec<u8>> {
        if !self.quests.is_empty() { return Err(Error::Invalid("legacy participant cannot carry quests")); }
        self.encode_schema(1)
    }
    /// A record from a live slot's maps. Sorting is canonicalisation, never
    /// loss: a count beyond a cap is a refusal naming the field.
    pub fn from_maps<'a>(
        context: RecordContext,
        reputation: impl IntoIterator<Item = (&'a (i32, i32), &'a i16)>,
        kills: impl IntoIterator<Item = (&'a MobType, &'a u32)>,
        claims: impl IntoIterator<Item = (&'a u32, &'a u32)>,
        local: Option<LocalRestore>,
    ) -> Result<Self> {
        let mut reputation: Vec<_> = reputation.into_iter().map(|(k, v)| (*k, *v)).collect();
        reputation.sort_by_key(|e| e.0);
        let mut kills = kills
            .into_iter()
            .map(|(k, v)| Ok((mob_index(*k)?, *k, *v)))
            .collect::<Result<Vec<_>>>()?;
        kills.sort_by_key(|e| e.0);
        let mut claims: Vec<_> = claims.into_iter().map(|(k, v)| (*k, *v)).collect();
        claims.sort_by_key(|e| e.0);
        let r = Self {
            context,
            reputation,
            kills: kills.into_iter().map(|(_, k, v)| (k, v)).collect(),
            claims,
            local,
            revision: 0,
            tracking: crate::progression::KillTracking::LegacySnapshot,
            quests: Default::default(),
        };
        r.validate()?;
        Ok(r)
    }
    /// True when this participant has no progression at all (pose aside).
    pub fn is_default(&self) -> bool {
        self.reputation.iter().all(|e| e.1 == 0)
            && self.kills.iter().all(|e| e.1 == 0)
            && self.claims.is_empty()
            && self.quests.is_empty()
    }
    fn validate(&self) -> Result<()> {
        if self.reputation.len() > REPUTATION_LIMIT {
            return Err(Error::Limit("participant reputation entries"));
        }
        if self.kills.len() > KILL_KINDS_LIMIT {
            return Err(Error::Limit("participant kill kinds"));
        }
        if self.claims.len() > CLAIMS_LIMIT {
            return Err(Error::Limit("participant bounty claims"));
        }
        if self.reputation.windows(2).any(|w| w[0].0 >= w[1].0)
            || self.claims.windows(2).any(|w| w[0].0 >= w[1].0)
        {
            return Err(Error::Invalid("participant duplicate key"));
        }
        let mut last = None;
        for (m, _) in &self.kills {
            let i = mob_index(*m)?;
            if last.is_some_and(|l| l >= i) {
                return Err(Error::Invalid("participant duplicate key"));
            }
            last = Some(i);
        }
        let (lo, hi) = (crate::reputation::REP_MIN, crate::reputation::REP_MAX);
        if self.reputation.iter().any(|e| e.1 < lo || e.1 > hi) {
            return Err(Error::Invalid("participant reputation score"));
        }
        if let Some(l) = &self.local {
            let finite = l
                .pos
                .iter()
                .chain([&l.yaw, &l.pitch])
                .all(|f| f.is_finite());
            if !finite || l.hotbar > 8 || l.locks & !LOCK_BITS != 0 {
                return Err(Error::Invalid("participant local restore"));
            }
        }
        self.quests.validate().map_err(|_| Error::Invalid("participant quests"))?;
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>> { self.encode_schema(SCHEMA) }
    fn encode_schema(&self, schema: u32) -> Result<Vec<u8>> {
        self.validate()?;
        let mut v = Vec::new();
        frame(&mut v, DOMAIN.as_bytes());
        v.extend_from_slice(&schema.to_be_bytes());
        v.extend_from_slice(&self.context.participant);
        v.extend_from_slice(&self.context.lineage);
        v.extend_from_slice(&self.context.generation.to_be_bytes());
        v.extend_from_slice(&self.context.transfer);
        v.extend_from_slice(&(self.reputation.len() as u32).to_be_bytes());
        for ((x, z), s) in &self.reputation {
            v.extend_from_slice(&x.to_be_bytes());
            v.extend_from_slice(&z.to_be_bytes());
            v.extend_from_slice(&s.to_be_bytes());
        }
        v.extend_from_slice(&(self.kills.len() as u32).to_be_bytes());
        for (m, n) in &self.kills {
            v.extend_from_slice(&mob_index(*m)?.to_be_bytes());
            v.extend_from_slice(&n.to_be_bytes());
        }
        v.extend_from_slice(&(self.claims.len() as u32).to_be_bytes());
        for (id, n) in &self.claims {
            v.extend_from_slice(&id.to_be_bytes());
            v.extend_from_slice(&n.to_be_bytes());
        }
        match &self.local {
            None => v.push(0),
            Some(l) => {
                v.push(1);
                for f in l.pos.iter().chain([&l.yaw, &l.pitch]) {
                    v.extend_from_slice(&f.to_bits().to_be_bytes());
                }
                v.push(l.hotbar);
                v.extend_from_slice(&l.locks.to_be_bytes());
            }
        }
        if schema == 2 {
            let extra = crate::progression::encode_extras(&self.to_gameplay())
                .map_err(|_| Error::Invalid("participant quest encoding"))?;
            frame(&mut v, &extra);
        }
        if v.len() > RECORD_LIMIT {
            return Err(Error::Limit("participant record"));
        }
        Ok(v)
    }
    /// Strict decode; every count is checked against its cap BEFORE any
    /// allocation, and the caller's expected context must match exactly.
    pub fn decode(bytes: &[u8], expected: &RecordContext) -> Result<Self> {
        let mut r = Reader::new(bytes, RECORD_LIMIT)?;
        r.label(DOMAIN)?;
        let schema = r.u32()?;
        if ![1, 2].contains(&schema) { return Err(Error::Invalid("participant schema")); }
        let context = RecordContext {
            participant: r.array()?,
            lineage: r.array()?,
            generation: r.u64()?,
            transfer: r.array()?,
        };
        if &context != expected {
            return Err(Error::Invalid("participant context"));
        }
        let count = |r: &mut Reader<'_>, cap: usize, what: &'static str| -> Result<usize> {
            let n = r.u32()? as usize;
            if n > cap {
                return Err(Error::Limit(what));
            }
            Ok(n)
        };
        let n = count(&mut r, REPUTATION_LIMIT, "participant reputation entries")?;
        let mut reputation = Vec::with_capacity(n);
        for _ in 0..n {
            let x = i32::from_be_bytes(r.array()?);
            let z = i32::from_be_bytes(r.array()?);
            reputation.push(((x, z), i16::from_be_bytes(r.array()?)));
        }
        let n = count(&mut r, KILL_KINDS_LIMIT, "participant kill kinds")?;
        let mut kills = Vec::with_capacity(n);
        for _ in 0..n {
            let m = mob_from_index(r.u32()?)?;
            kills.push((m, r.u32()?));
        }
        let n = count(&mut r, CLAIMS_LIMIT, "participant bounty claims")?;
        let mut claims = Vec::with_capacity(n);
        for _ in 0..n {
            claims.push((r.u32()?, r.u32()?));
        }
        let local = match r.u8()? {
            0 => None,
            1 => {
                let mut f = [0f32; 5];
                for x in &mut f {
                    *x = f32::from_bits(r.u32()?);
                }
                Some(LocalRestore {
                    pos: [f[0], f[1], f[2]],
                    yaw: f[3],
                    pitch: f[4],
                    hotbar: r.u8()?,
                    locks: r.u64()?,
                })
            }
            _ => return Err(Error::Invalid("participant flags")),
        };
        let (revision, tracking, quests) = if schema == 2 {
            crate::progression::decode_extras(r.framed(RECORD_LIMIT)?)
                .map_err(|_| Error::Invalid("participant quest suffix"))?
        } else { (0, crate::progression::KillTracking::LegacySnapshot, Default::default()) };
        r.end()?;
        let rec = Self {
            context,
            reputation,
            kills,
            claims,
            local, revision, tracking, quests,
        };
        rec.validate()?;
        Ok(rec)
    }
    /// Install ONLY this record's progression into a slot that belongs to
    /// the same participant. No reward, claim or payout routine runs.
    pub(crate) fn install_progression(&self, slot: &mut crate::player_slot::PlayerSlot) {
        slot.reputation.per_village.clear();
        for (k, s) in &self.reputation {
            slot.reputation.per_village.insert(*k, *s);
        }
        slot.kill_counter.clear();
        for (m, n) in &self.kills {
            slot.kill_counter.insert(*m, *n);
        }
        slot.bounties_claimed.clear();
        for (id, n) in &self.claims {
            slot.bounties_claimed.insert(*id, *n);
        }
    }
}

#[cfg(test)]
mod tests;
