//! Counts are checked before allocation; reward fields cannot decode private bodies.
use super::*;
use bincode::Options;

const DOMAIN: &[u8] = b"AxeNStax/gameplay/progression/v1";
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, String> {
        Self::with_limit(bytes, RECORD_LIMIT)
    }
    pub(crate) fn with_limit(bytes: &'a [u8], limit: usize) -> Result<Self, String> {
        if bytes.len() > limit {
            return Err("bounded value exceeds its byte limit".into());
        }
        Ok(Self { bytes, at: 0 })
    }
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .ok_or("gameplay offset overflow")?;
        let value = self
            .bytes
            .get(self.at..end)
            .ok_or("truncated gameplay record")?;
        self.at = end;
        Ok(value)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?
            .try_into()
            .map_err(|_| "invalid fixed gameplay field".into())
    }
    pub(crate) fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }
    pub(crate) fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub(crate) fn count(&mut self, cap: usize) -> Result<usize, String> {
        let count = self.u32()? as usize;
        if count > cap {
            return Err("gameplay field exceeds its entry limit".into());
        }
        Ok(count)
    }
    pub(crate) fn frame(&mut self, cap: usize) -> Result<&'a [u8], String> {
        let count = self.count(cap)?;
        self.take(count)
    }
    pub(crate) fn fixed<T: serde::de::DeserializeOwned>(
        &mut self,
        cap: usize,
    ) -> Result<T, String> {
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(cap as u64)
            .reject_trailing_bytes()
            .deserialize(self.frame(cap)?)
            .map_err(|e| e.to_string())
    }
    pub(crate) fn end(self) -> Result<(), String> {
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err("trailing gameplay bytes".into())
        }
    }
}
fn frame(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    let size = u32::try_from(bytes.len()).map_err(|_| "gameplay field length overflow")?;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}
fn fixed<T: Serialize>(out: &mut Vec<u8>, value: &T) -> Result<(), String> {
    frame(out, &bincode::serialize(value).map_err(|e| e.to_string())?)
}
fn count(out: &mut Vec<u8>, value: usize) -> Result<(), String> {
    out.extend_from_slice(
        &u32::try_from(value)
            .map_err(|_| "gameplay count overflow")?
            .to_be_bytes(),
    );
    Ok(())
}
fn tracking(out: &mut Vec<u8>, value: KillTracking) {
    out.push(match value {
        KillTracking::LegacyUntracked => 0,
        KillTracking::Authoritative => 1,
        KillTracking::LegacySnapshot => 2,
    });
}
fn read_tracking(reader: &mut Reader<'_>) -> Result<KillTracking, String> {
    match reader.u8()? {
        0 => Ok(KillTracking::LegacyUntracked),
        1 => Ok(KillTracking::Authoritative),
        2 => Ok(KillTracking::LegacySnapshot),
        _ => Err("unknown gameplay tracking provenance".into()),
    }
}
fn quests(out: &mut Vec<u8>, value: &QuestProgress) -> Result<(), String> {
    count(out, value.accepted.len())?;
    for quest in &value.accepted {
        out.extend_from_slice(&quest.id);
        out.extend_from_slice(&quest.npc);
        out.extend_from_slice(&quest.village.0.to_be_bytes());
        out.extend_from_slice(&quest.village.1.to_be_bytes());
        fixed(out, &quest.flavour)?;
        count(out, quest.rewards.len())?;
        for reward in &quest.rewards {
            fixed(out, reward)?;
        }
        out.extend_from_slice(&quest.reputation.to_be_bytes());
        out.extend_from_slice(&quest.baseline.to_be_bytes());
    }
    count(out, value.declines.len())?;
    for (npc, ticks) in &value.declines {
        out.extend_from_slice(npc);
        out.extend_from_slice(&ticks.to_be_bytes());
    }
    out.extend_from_slice(&value.penalty_left.to_be_bytes());
    Ok(())
}
fn read_quests(reader: &mut Reader<'_>) -> Result<QuestProgress, String> {
    let count = reader.count(QUEST_LIMIT)?;
    let mut accepted = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.array()?;
        let npc = reader.array()?;
        let village = (
            i32::from_be_bytes(reader.array()?),
            i32::from_be_bytes(reader.array()?),
        );
        let flavour = reader.fixed(32)?;
        let count = reader.count(REWARD_LIMIT)?;
        let mut rewards = Vec::with_capacity(count);
        for _ in 0..count {
            let bytes = reader.frame(32)?;
            // Only SavedItemSlot::Saved with a scalar SavedSlot variant is
            // eligible. Refuse Plan before serde can allocate its body.
            let outer = bytes
                .get(..4)
                .and_then(|v| v.try_into().ok())
                .map(u32::from_le_bytes);
            let inner = bytes
                .get(4..8)
                .and_then(|v| v.try_into().ok())
                .map(u32::from_le_bytes);
            if outer != Some(0) || !matches!(inner, Some(1 | 2 | 3 | 5)) {
                return Err("invalid or private quest reward".into());
            }
            rewards.push(
                bincode::DefaultOptions::new()
                    .with_fixint_encoding()
                    .with_limit(32)
                    .reject_trailing_bytes()
                    .deserialize(bytes)
                    .map_err(|e| e.to_string())?,
            );
        }
        accepted.push(AcceptedQuest {
            id,
            npc,
            village,
            flavour,
            rewards,
            reputation: i16::from_be_bytes(reader.array()?),
            baseline: reader.u32()?,
        });
    }
    let count = reader.count(DECLINE_LIMIT)?;
    let mut declines = Vec::with_capacity(count);
    for _ in 0..count {
        declines.push((reader.array()?, reader.u32()?));
    }
    let value = QuestProgress {
        accepted,
        declines,
        penalty_left: reader.u32()?,
    };
    value.validate()?;
    Ok(value)
}
pub(super) fn encode(record: &GameplayRecord) -> Result<Vec<u8>, String> {
    record.validate()?;
    let mut out = Vec::new();
    frame(&mut out, DOMAIN)?;
    fixed(&mut out, &record.context)?;
    out.extend_from_slice(&record.revision.to_be_bytes());
    tracking(&mut out, record.tracking);
    count(&mut out, record.reputation.len())?;
    for ((x, z), score) in &record.reputation {
        out.extend_from_slice(&x.to_be_bytes());
        out.extend_from_slice(&z.to_be_bytes());
        out.extend_from_slice(&score.to_be_bytes());
    }
    count(&mut out, record.kills.len())?;
    for (kind, total) in &record.kills {
        fixed(&mut out, kind)?;
        out.extend_from_slice(&total.to_be_bytes());
    }
    count(&mut out, record.claims.len())?;
    for (id, total) in &record.claims {
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&total.to_be_bytes());
    }
    match record.local {
        Some(local) => {
            out.push(1);
            fixed(&mut out, &local)?;
        }
        None => out.push(0),
    }
    quests(&mut out, &record.quests)?;
    if out.len() > RECORD_LIMIT {
        return Err("gameplay record exceeds 64 KiB".into());
    }
    Ok(out)
}
pub(super) fn decode(bytes: &[u8], expected: GameplayContext) -> Result<GameplayRecord, String> {
    let mut reader = Reader::new(bytes)?;
    if reader.frame(DOMAIN.len())? != DOMAIN {
        return Err("unknown gameplay record domain".into());
    }
    let context = reader.fixed(128)?;
    if context != expected {
        return Err("gameplay record belongs to another identity or attachment".into());
    }
    let revision = reader.u64()?;
    let tracking = read_tracking(&mut reader)?;
    let count = reader.count(REPUTATION_LIMIT)?;
    let mut reputation = Vec::with_capacity(count);
    for _ in 0..count {
        reputation.push((
            (
                i32::from_be_bytes(reader.array()?),
                i32::from_be_bytes(reader.array()?),
            ),
            i16::from_be_bytes(reader.array()?),
        ));
    }
    let count = reader.count(KILL_KINDS_LIMIT)?;
    let mut kills = Vec::with_capacity(count);
    for _ in 0..count {
        kills.push((reader.fixed(4)?, reader.u32()?));
    }
    let count = reader.count(CLAIMS_LIMIT)?;
    let mut claims = Vec::with_capacity(count);
    for _ in 0..count {
        claims.push((reader.u32()?, reader.u32()?));
    }
    let local = match reader.u8()? {
        0 => None,
        1 => Some(reader.fixed(64)?),
        _ => return Err("invalid local gameplay flag".into()),
    };
    let quests = read_quests(&mut reader)?;
    reader.end()?;
    let value = GameplayRecord {
        context,
        revision,
        reputation,
        kills,
        claims,
        local,
        tracking,
        quests,
    };
    value.validate()?;
    Ok(value)
}

/// The schema2 participant suffix carries only fields schema1 did not have.
pub(super) fn encode_extras(record: &GameplayRecord) -> Result<Vec<u8>, String> {
    record.quests.validate()?;
    let mut out = Vec::new();
    out.extend_from_slice(&record.revision.to_be_bytes());
    tracking(&mut out, record.tracking);
    quests(&mut out, &record.quests)?;
    Ok(out)
}
pub(super) fn decode_extras(bytes: &[u8]) -> Result<(u64, KillTracking, QuestProgress), String> {
    let mut reader = Reader::new(bytes)?;
    let revision = reader.u64()?;
    let tracking = read_tracking(&mut reader)?;
    let quests = read_quests(&mut reader)?;
    reader.end()?;
    Ok((revision, tracking, quests))
}

/// The genuinely delivered schema1 record is portable data, not a native
/// proof operation. Its exact old domain, context and byte order are retained.
pub(super) fn decode_schema1(
    bytes: &[u8],
    expected: PersonalContext,
) -> Result<GameplayRecord, String> {
    let mut reader = Reader::new(bytes)?;
    let domain = b"AxeNStax/handover/participant/v1";
    if reader.frame(domain.len())? != domain || reader.u32()? != 1 {
        return Err("legacy progression schema differs".into());
    }
    let context = PersonalContext {
        participant: reader.array()?,
        lineage: reader.array()?,
        generation: reader.u64()?,
        transfer: reader.array()?,
    };
    if context != expected {
        return Err("legacy progression context differs".into());
    }
    let count = reader.count(REPUTATION_LIMIT)?;
    let mut reputation = Vec::with_capacity(count);
    for _ in 0..count {
        reputation.push((
            (
                i32::from_be_bytes(reader.array()?),
                i32::from_be_bytes(reader.array()?),
            ),
            i16::from_be_bytes(reader.array()?),
        ));
    }
    let count = reader.count(KILL_KINDS_LIMIT)?;
    let mut kills = Vec::with_capacity(count);
    for _ in 0..count {
        let index = reader.u32()?;
        let kind = bincode::deserialize(&index.to_le_bytes()).map_err(|e| e.to_string())?;
        kills.push((kind, reader.u32()?));
    }
    let count = reader.count(CLAIMS_LIMIT)?;
    let mut claims = Vec::with_capacity(count);
    for _ in 0..count {
        claims.push((reader.u32()?, reader.u32()?));
    }
    let local = match reader.u8()? {
        0 => None,
        1 => {
            let mut values = [0f32; 5];
            for value in &mut values {
                *value = f32::from_bits(reader.u32()?);
            }
            Some(LocalRestore {
                pos: [values[0], values[1], values[2]],
                yaw: values[3],
                pitch: values[4],
                hotbar: reader.u8()?,
                locks: reader.u64()?,
            })
        }
        _ => return Err("invalid legacy local restore flag".into()),
    };
    reader.end()?;
    let record = GameplayRecord {
        context: GameplayContext::Imported(context),
        revision: 0,
        reputation,
        kills,
        claims,
        local,
        tracking: KillTracking::LegacySnapshot,
        quests: Default::default(),
    };
    record.validate()?;
    Ok(record)
}
