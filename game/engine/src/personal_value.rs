//! Portable personal value codecs, with no native authority or gameplay APIs.
use crate::joiner_save::MAX_JOINER_SAVE_BYTES;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalContext {
    pub participant: [u8; 32],
    pub lineage: [u8; 16],
    pub generation: u64,
    pub transfer: [u8; 16],
}

/// The selected world's local personal snapshot uses a known canonical v3/v4
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
                if !matches!(bytes.first(), Some(3 | 4)) {
                    return Err(serde::de::Error::custom(
                        "personal snapshot has an unsupported canonical version",
                    ));
                }
                Ok(LocalPersonalSave(bytes))
            }
        }
        d.deserialize_seq(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_portable_context_and_snapshot_pin_the_exact_old_codec_shape() {
        let context = PersonalContext {
            participant: [1; 32],
            lineage: [2; 16],
            generation: 3,
            transfer: [4; 16],
        };
        let mut literal = vec![1; 32];
        literal.extend_from_slice(&[2; 16]);
        literal.extend_from_slice(&3u64.to_le_bytes());
        literal.extend_from_slice(&[4; 16]);
        assert_eq!(
            bincode::serialize(&context).unwrap(),
            literal,
            "portable context reordered a saved/wire field"
        );
        assert_eq!(
            bincode::deserialize::<PersonalContext>(&literal).unwrap(),
            context
        );
        let bytes = vec![3, 7, 9];
        let encoded = bincode::serialize(&LocalPersonalSave(bytes.clone())).unwrap();
        assert_eq!(
            encoded,
            bincode::serialize(&bytes).unwrap(),
            "boxed local value changed the canonical byte carrier"
        );
        let mut expected = 3u64.to_le_bytes().to_vec();
        expected.extend_from_slice(&bytes);
        assert_eq!(encoded, expected);
        assert_eq!(
            bincode::deserialize::<LocalPersonalSave>(&encoded)
                .unwrap()
                .0,
            bytes
        );
    }

    #[test]
    fn personal_local_snapshot_refuses_oversized_hint_and_unknown_version() {
        let huge = (MAX_JOINER_SAVE_BYTES + 1).to_le_bytes();
        assert!(
            bincode::deserialize::<LocalPersonalSave>(&huge).is_err(),
            "oversized sequence reached allocation"
        );
        for bytes in [vec![], vec![1], vec![2], vec![4]] {
            assert!(
                bincode::deserialize::<LocalPersonalSave>(&bincode::serialize(&bytes).unwrap())
                    .is_err(),
                "unknown local record became an absent baseline"
            );
        }
    }
}
