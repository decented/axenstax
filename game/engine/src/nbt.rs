//! Minimal big-endian NBT reader (#10) — just enough to parse Sponge `.schem`
//! schematics. Hand-rolled (no new crate); gzip de-compression is the caller's
//! job (`flate2`, already a dependency). All headless-unit-tested over
//! hand-built byte blobs.
//!
//! Spec: `docs/foundations/2026-06-16-schematic-import.md`.

use std::collections::HashMap;

/// An NBT tag value.
#[derive(Clone, Debug, PartialEq)]
pub enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Nbt>),
    Compound(HashMap<String, Nbt>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

#[derive(Debug, PartialEq, Eq)]
pub enum NbtError {
    /// Ran off the end of the buffer.
    Eof,
    /// Unknown tag id.
    BadTag(u8),
    /// A string payload wasn't valid UTF-8.
    BadUtf8,
    /// Lists/compounds nested deeper than [`MAX_DEPTH`].
    TooDeep,
    /// A length that can't fit in the remaining bytes, or a document whose
    /// decoded size would pass [`MAX_ALLOC_BYTES`].
    TooLarge,
}

/// Deepest List/Compound nesting accepted. Real Sponge schematics nest a
/// handful of levels; a crafted file nesting thousands (≈5 bytes a level) would
/// otherwise overflow the stack — an abort, not a catchable panic.
pub const MAX_DEPTH: u32 = 64;

/// Ceiling on the memory one document may decode into (audit 2026-09-27).
pub const MAX_ALLOC_BYTES: usize = 256 * 1024 * 1024;

/// Smallest encoded size of one payload of `tag`, used to bound a declared
/// list length by the bytes that remain. `None` for an unknown tag.
fn min_payload_len(tag: u8) -> Option<usize> {
    Some(match tag {
        1 => 1,
        2 => 2,
        3 | 5 => 4,
        4 | 6 => 8,
        7 | 11 | 12 => 4, // i32 length prefix
        8 => 2,           // u16 length prefix
        9 => 5,           // element tag + i32 length
        10 => 1,          // End tag
        _ => return None,
    })
}

impl Nbt {
    pub fn as_compound(&self) -> Option<&HashMap<String, Nbt>> {
        match self {
            Nbt::Compound(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_i16(&self) -> Option<i16> {
        match self {
            Nbt::Short(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Nbt::Int(v) => Some(*v),
            _ => None,
        }
    }
    /// Unlike `as_i32` above (live in schematic.rs), no production caller
    /// reads a String tag back — tested directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Nbt::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_byte_array(&self) -> Option<&[i8]> {
        match self {
            Nbt::ByteArray(b) => Some(b),
            _ => None,
        }
    }
    /// Convenience: look up `key` in a compound.
    pub fn get(&self, key: &str) -> Option<&Nbt> {
        self.as_compound().and_then(|m| m.get(key))
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
    /// Current List/Compound nesting.
    depth: u32,
    /// Bytes of decoded storage charged so far (see [`MAX_ALLOC_BYTES`]).
    allocated: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.b.len() - self.i
    }
    /// Charge `n` bytes of decoded storage against the document budget.
    fn charge(&mut self, n: usize) -> Result<(), NbtError> {
        self.allocated = self.allocated.checked_add(n).ok_or(NbtError::TooLarge)?;
        if self.allocated > MAX_ALLOC_BYTES {
            return Err(NbtError::TooLarge);
        }
        Ok(())
    }
    /// Read an i32 element count and check `count * min_elem` fits in the
    /// bytes that remain — so no allocation is ever sized from an untrusted
    /// length the input can't back.
    fn count(&mut self, min_elem: usize) -> Result<usize, NbtError> {
        let len = self.i32()?.max(0) as usize;
        if len.checked_mul(min_elem).is_none_or(|n| n > self.remaining()) {
            return Err(NbtError::TooLarge);
        }
        Ok(len)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], NbtError> {
        if self.i + n > self.b.len() {
            return Err(NbtError::Eof);
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, NbtError> {
        let s = self.take(2)?;
        Ok(i16::from_be_bytes([s[0], s[1]]))
    }
    fn u16(&mut self) -> Result<u16, NbtError> {
        Ok(self.i16()? as u16)
    }
    fn i32(&mut self) -> Result<i32, NbtError> {
        let s = self.take(4)?;
        Ok(i32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i64(&mut self) -> Result<i64, NbtError> {
        let s = self.take(8)?;
        Ok(i64::from_be_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }
    fn f32(&mut self) -> Result<f32, NbtError> {
        Ok(f32::from_bits(self.i32()? as u32))
    }
    fn f64(&mut self) -> Result<f64, NbtError> {
        Ok(f64::from_bits(self.i64()? as u64))
    }
    fn string(&mut self) -> Result<String, NbtError> {
        let len = self.u16()? as usize;
        let s = self.take(len)?;
        std::str::from_utf8(s)
            .map(|v| v.to_string())
            .map_err(|_| NbtError::BadUtf8)
    }
    /// Step one List/Compound level deeper, refusing past [`MAX_DEPTH`].
    fn enter(&mut self) -> Result<(), NbtError> {
        if self.depth >= MAX_DEPTH {
            return Err(NbtError::TooDeep);
        }
        self.depth += 1;
        Ok(())
    }
    fn payload(&mut self, tag: u8) -> Result<Nbt, NbtError> {
        Ok(match tag {
            1 => Nbt::Byte(self.u8()? as i8),
            2 => Nbt::Short(self.i16()?),
            3 => Nbt::Int(self.i32()?),
            4 => Nbt::Long(self.i64()?),
            5 => Nbt::Float(self.f32()?),
            6 => Nbt::Double(self.f64()?),
            7 => {
                let len = self.count(1)?;
                self.charge(len)?;
                let s = self.take(len)?;
                Nbt::ByteArray(s.iter().map(|&b| b as i8).collect())
            }
            8 => {
                let s = self.string()?;
                self.charge(s.len())?;
                Nbt::String(s)
            }
            9 => {
                let elem = self.u8()?;
                // An empty list may carry the End tag (0) as its element type.
                let min = if elem == 0 { 0 } else { min_payload_len(elem).ok_or(NbtError::BadTag(elem))? };
                let len = self.count(min)?;
                if elem == 0 && len > 0 {
                    return Err(NbtError::BadTag(0));
                }
                self.charge(len.saturating_mul(std::mem::size_of::<Nbt>()))?;
                self.enter()?;
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    v.push(self.payload(elem)?);
                }
                self.depth -= 1;
                Nbt::List(v)
            }
            10 => {
                self.enter()?;
                let mut m = HashMap::new();
                loop {
                    let t = self.u8()?;
                    if t == 0 {
                        break; // End tag
                    }
                    let name = self.string()?;
                    self.charge(name.len() + std::mem::size_of::<Nbt>())?;
                    let val = self.payload(t)?;
                    m.insert(name, val);
                }
                self.depth -= 1;
                Nbt::Compound(m)
            }
            11 => {
                let len = self.count(4)?;
                self.charge(len * 4)?;
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    v.push(self.i32()?);
                }
                Nbt::IntArray(v)
            }
            12 => {
                let len = self.count(8)?;
                self.charge(len * 8)?;
                let mut v = Vec::with_capacity(len);
                for _ in 0..len {
                    v.push(self.i64()?);
                }
                Nbt::LongArray(v)
            }
            other => return Err(NbtError::BadTag(other)),
        })
    }
}

/// Parse a root NBT document → `(root name, root tag)`. The caller gunzips first.
pub fn parse(bytes: &[u8]) -> Result<(String, Nbt), NbtError> {
    let mut r = Reader { b: bytes, i: 0, depth: 0, allocated: 0 };
    let tag = r.u8()?;
    if tag == 0 {
        return Err(NbtError::Eof); // an empty document has no root
    }
    let name = r.string()?;
    let payload = r.payload(tag)?;
    Ok((name, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_compound_with_scalar_children() {
        // Compound "T" { Int x = 5, Short y = -2 }
        let bytes = [
            10, 0, 1, b'T', // Compound, name "T"
            3, 0, 1, b'x', 0, 0, 0, 5, // Int x = 5
            2, 0, 1, b'y', 0xff, 0xfe, // Short y = -2
            0, // End
        ];
        let (name, root) = parse(&bytes).unwrap();
        assert_eq!(name, "T");
        assert_eq!(root.get("x"), Some(&Nbt::Int(5)));
        assert_eq!(root.get("y"), Some(&Nbt::Short(-2)));
    }

    #[test]
    fn parses_strings_byte_arrays_and_lists() {
        // Compound "" { String s = "hi", ByteArray b = [1,2,3], List<Int> l = [7,8] }
        let bytes = [
            10, 0, 0, // Compound, empty name
            8, 0, 1, b's', 0, 2, b'h', b'i', // String s = "hi"
            7, 0, 1, b'b', 0, 0, 0, 3, 1, 2, 3, // ByteArray b = [1,2,3]
            9, 0, 1, b'l', 3, 0, 0, 0, 2, 0, 0, 0, 7, 0, 0, 0, 8, // List<Int> l=[7,8]
            0, // End
        ];
        let (_, root) = parse(&bytes).unwrap();
        assert_eq!(root.get("s").and_then(|t| t.as_string()), Some("hi"));
        assert_eq!(root.get("b").and_then(|t| t.as_byte_array()), Some(&[1i8, 2, 3][..]));
        assert_eq!(
            root.get("l"),
            Some(&Nbt::List(vec![Nbt::Int(7), Nbt::Int(8)]))
        );
    }

    #[test]
    fn truncated_input_is_eof_not_a_panic() {
        let bytes = [10, 0, 1, b'T', 3, 0, 1, b'x', 0, 0]; // Int payload cut short
        assert_eq!(parse(&bytes), Err(NbtError::Eof));
    }

    #[test]
    fn unknown_tag_is_reported() {
        let bytes = [10, 0, 0, 99, 0, 1, b'z']; // tag 99 doesn't exist
        assert_eq!(parse(&bytes), Err(NbtError::BadTag(99)));
    }

    // ── Audit 2026-09-27: crafted `.schem` must error, never abort ──

    #[test]
    fn a_huge_list_length_is_rejected_not_allocated() {
        // Compound "" { List<Compound> l, len 0x7fffffff, no elements }
        let bytes = [10, 0, 0, 9, 0, 1, b'l', 10, 0x7f, 0xff, 0xff, 0xff];
        assert_eq!(parse(&bytes), Err(NbtError::TooLarge));
    }

    #[test]
    fn huge_int_and_long_array_lengths_are_rejected() {
        for tag in [11u8, 12] {
            let bytes = [10, 0, 0, tag, 0, 1, b'a', 0x7f, 0xff, 0xff, 0xff, 1, 2, 3, 4];
            assert_eq!(parse(&bytes), Err(NbtError::TooLarge), "tag {tag}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_stack_overflow() {
        // 100 000 nested single-element lists of lists: ≈5 bytes a level.
        let mut bytes = vec![9, 0, 0]; // root List, empty name
        for _ in 0..100_000 {
            bytes.extend_from_slice(&[9, 0, 0, 0, 1]); // elem=List, len=1
        }
        bytes.extend_from_slice(&[1, 0, 0, 0, 0]); // innermost: List<Byte> len 0
        assert_eq!(parse(&bytes), Err(NbtError::TooDeep));
    }

    #[test]
    fn nesting_up_to_the_limit_still_parses() {
        let mut bytes = vec![9, 0, 0];
        for _ in 0..(MAX_DEPTH - 2) {
            bytes.extend_from_slice(&[9, 0, 0, 0, 1]);
        }
        bytes.extend_from_slice(&[1, 0, 0, 0, 0]);
        assert!(parse(&bytes).is_ok());
    }
}
