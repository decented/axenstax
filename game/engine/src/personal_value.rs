//! Portable personal value codecs, with no native authority or gameplay APIs.
use serde::{Deserialize, Serialize};
use crate::joiner_save::MAX_JOINER_SAVE_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalContext {
    pub participant: [u8; 32],
    pub lineage: [u8; 16],
    pub generation: u64,
    pub transfer: [u8; 16],
}


/// The selected world's local personal snapshot uses the exact canonical v3
/// codec, bounded before allocation. It commits with that world's world.dat.
#[derive(Clone, Serialize)]
pub struct LocalPersonalSave(pub(crate) Vec<u8>);
impl<'de> Deserialize<'de> for LocalPersonalSave {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = LocalPersonalSave;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("one bounded canonical personal record")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let hint = seq.size_hint().unwrap_or(0);
                if hint > MAX_JOINER_SAVE_BYTES as usize {
                    return Err(serde::de::Error::custom("personal snapshot exceeds 64 KiB"));
                }
                let mut bytes = Vec::with_capacity(hint);
                while let Some(byte) = seq.next_element::<u8>()? {
                    if bytes.len() >= MAX_JOINER_SAVE_BYTES as usize {
                        return Err(serde::de::Error::custom("personal snapshot exceeds 64 KiB"));
                    }
                    bytes.push(byte);
                }
                if bytes.first() != Some(&3) {
                    return Err(serde::de::Error::custom(
                        "personal snapshot is not canonical v3",
                    ));
                }
                Ok(LocalPersonalSave(bytes))
            }
        }
        d.deserialize_seq(V)
    }
}
