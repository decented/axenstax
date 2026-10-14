//! No unbounded component strings or optional commission bodies enter this carrier.
use super::*;
use crate::progression::codec::Reader;
const DOMAIN: &[u8] = b"AxeNStax/quest/world/v1";
fn frame(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    out.extend_from_slice(
        &u32::try_from(bytes.len())
            .map_err(|_| "quest world length overflow")?
            .to_be_bytes(),
    );
    out.extend_from_slice(bytes);
    Ok(())
}
fn scalar<T: Serialize>(out: &mut Vec<u8>, value: &T) -> Result<(), String> {
    frame(out, &bincode::serialize(value).map_err(|e| e.to_string())?)
}
pub(super) fn encode(value: &QuestWorldSave) -> Result<Vec<u8>, String> {
    value.validate()?;
    let mut out = Vec::new();
    frame(&mut out, DOMAIN)?;
    out.extend_from_slice(&value.scope);
    out.extend_from_slice(&(value.npcs.len() as u32).to_be_bytes());
    for npc in &value.npcs {
        out.extend_from_slice(&npc.id);
        scalar(&mut out, &npc.kind)?;
        for number in npc
            .pos
            .iter()
            .chain(npc.velocity.iter())
            .chain(npc.health.iter())
        {
            out.extend_from_slice(&number.to_bits().to_be_bytes());
        }
        for ticks in npc.health_ticks {
            out.extend_from_slice(&ticks.to_be_bytes());
        }
        scalar(&mut out, &npc.component.profession)?;
        scalar(&mut out, &npc.component.claimed_workstation)?;
        match &npc.component.gossip_line {
            None => out.push(0),
            Some(line) => {
                out.push(1);
                frame(&mut out, line.as_bytes())?;
            }
        }
        out.extend_from_slice(&npc.component.gossip_last_refresh_tick.to_be_bytes());
        out.push(u8::from(npc.authored));
    }
    if out.len() > WORLD_CARRIER_LIMIT {
        return Err("quest world exceeds its byte limit".into());
    }
    Ok(out)
}
pub(super) fn decode(bytes: &[u8]) -> Result<QuestWorldSave, String> {
    let mut reader = Reader::with_limit(bytes, WORLD_CARRIER_LIMIT)?;
    if reader.frame(DOMAIN.len())? != DOMAIN {
        return Err("unknown quest world domain".into());
    }
    let scope = reader.array()?;
    let count = reader.count(NPC_LIMIT)?;
    let mut npcs = Vec::with_capacity(count);
    for _ in 0..count {
        let id = reader.array()?;
        let kind = reader.fixed(4)?;
        let mut numbers = [0f32; 8];
        for number in &mut numbers {
            *number = f32::from_bits(reader.u32()?);
        }
        let health_ticks = [reader.u32()?, reader.u32()?];
        let profession = reader.fixed(4)?;
        let claimed_workstation = reader.fixed(32)?;
        let gossip_line = match reader.u8()? {
            0 => None,
            1 => Some(
                String::from_utf8(reader.frame(GOSSIP_LIMIT)?.to_vec())
                    .map_err(|_| "invalid quest giver gossip")?,
            ),
            _ => return Err("invalid quest giver gossip flag".into()),
        };
        let gossip_last_refresh_tick = reader.u64()?;
        let authored = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err("invalid authored quest giver flag".into()),
        };
        npcs.push(SavedQuestNpc {
            id,
            kind,
            pos: [numbers[0], numbers[1], numbers[2]],
            velocity: [numbers[3], numbers[4], numbers[5]],
            health: [numbers[6], numbers[7]],
            health_ticks,
            component: VillagerComponent {
                profession,
                claimed_workstation,
                gossip_line,
                gossip_last_refresh_tick,
                commission: None,
            },
            authored,
        });
    }
    reader.end()?;
    let value = QuestWorldSave { scope, npcs };
    value.validate()?;
    Ok(value)
}
