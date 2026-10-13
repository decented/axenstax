//! Strict positional decoding with a separate conservative allocation budget.
//! Sequence hints are suppressed BEFORE Vec allocation; every element is charged
//! at four times its inline size plus node overhead before its seed is run. Borrowed
//! strings/bytes are charged before an owned visitor copies them. This bounds
//! collection capacity growth and map/tree overhead, including zero-sized input.
use super::super::{Error, Result};
use bincode::Options;
use serde::Deserialize;
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor,
};
use std::{cell::RefCell, fmt};
pub const WORLD_BYTES: usize = 64 * 1024 * 1024;
const RESIDENT: usize = 128 * 1024 * 1024;
struct Budget {
    remaining: usize,
    depth: usize,
}
impl Budget {
    fn charge<E: de::Error>(&mut self, bytes: usize) -> std::result::Result<(), E> {
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or_else(|| E::custom("checkpoint allocation cap"))?;
        Ok(())
    }
}
struct B<'a, D> {
    d: D,
    b: &'a RefCell<Budget>,
}
struct V<'a, T> {
    v: T,
    b: &'a RefCell<Budget>,
}
struct Seed<'a, S> {
    s: S,
    b: &'a RefCell<Budget>,
}
impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for Seed<'_, S> {
    type Value = S::Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        self.s.deserialize(B { d, b: self.b })
    }
}
struct Depth<'a>(&'a RefCell<Budget>);
impl Drop for Depth<'_> {
    fn drop(&mut self) {
        self.0.borrow_mut().depth -= 1;
    }
}
impl<'a, D> B<'a, D> {
    fn enter<E: de::Error>(&self) -> std::result::Result<Depth<'a>, E> {
        let mut b = self.b.borrow_mut();
        if b.depth >= 128 {
            return Err(E::custom("checkpoint nesting cap"));
        }
        b.depth += 1;
        Ok(Depth(self.b))
    }
}
macro_rules! forward {
    ($($method:ident),* $(,)?) => {$ (
        fn $method<T:Visitor<'de>>(self,v:T)->std::result::Result<T::Value,Self::Error> {
            let _depth=self.enter()?;self.d.$method(V{v,b:self.b})
        }
    )*};
}
impl<'de, D: Deserializer<'de>> Deserializer<'de> for B<'_, D> {
    type Error = D::Error;
    forward!(
        deserialize_any,
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char
    );
    fn deserialize_str<T: Visitor<'de>>(self, v: T) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_str(V { v, b: self.b })
    }
    fn deserialize_string<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_str(V { v, b: self.b })
    }
    fn deserialize_bytes<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_bytes(V { v, b: self.b })
    }
    fn deserialize_byte_buf<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_bytes(V { v, b: self.b })
    }
    fn deserialize_option<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_option(V { v, b: self.b })
    }
    fn deserialize_unit<T: Visitor<'de>>(self, v: T) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_unit(V { v, b: self.b })
    }
    fn deserialize_seq<T: Visitor<'de>>(self, v: T) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_seq(V { v, b: self.b })
    }
    fn deserialize_map<T: Visitor<'de>>(self, v: T) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_map(V { v, b: self.b })
    }
    fn deserialize_identifier<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_identifier(V { v, b: self.b })
    }
    fn deserialize_ignored_any<T: Visitor<'de>>(
        self,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_ignored_any(V { v, b: self.b })
    }
    fn deserialize_unit_struct<T: Visitor<'de>>(
        self,
        name: &'static str,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_unit_struct(name, V { v, b: self.b })
    }
    fn deserialize_newtype_struct<T: Visitor<'de>>(
        self,
        name: &'static str,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_newtype_struct(name, V { v, b: self.b })
    }
    fn deserialize_tuple<T: Visitor<'de>>(
        self,
        len: usize,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_tuple(len, V { v, b: self.b })
    }
    fn deserialize_tuple_struct<T: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d
            .deserialize_tuple_struct(name, len, V { v, b: self.b })
    }
    fn deserialize_struct<T: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_struct(name, fields, V { v, b: self.b })
    }
    fn deserialize_enum<T: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        let _depth = self.enter()?;
        self.d.deserialize_enum(name, variants, V { v, b: self.b })
    }
    fn is_human_readable(&self) -> bool {
        self.d.is_human_readable()
    }
}
impl<'de, T: Visitor<'de>> Visitor<'de> for V<'_, T> {
    type Value = T::Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.v.expecting(f)
    }
    fn visit_bool<E: de::Error>(self, x: bool) -> std::result::Result<Self::Value, E> {
        self.v.visit_bool(x)
    }
    fn visit_i8<E: de::Error>(self, x: i8) -> std::result::Result<Self::Value, E> {
        self.v.visit_i8(x)
    }
    fn visit_i16<E: de::Error>(self, x: i16) -> std::result::Result<Self::Value, E> {
        self.v.visit_i16(x)
    }
    fn visit_i32<E: de::Error>(self, x: i32) -> std::result::Result<Self::Value, E> {
        self.v.visit_i32(x)
    }
    fn visit_i64<E: de::Error>(self, x: i64) -> std::result::Result<Self::Value, E> {
        self.v.visit_i64(x)
    }
    fn visit_i128<E: de::Error>(self, x: i128) -> std::result::Result<Self::Value, E> {
        self.v.visit_i128(x)
    }
    fn visit_u8<E: de::Error>(self, x: u8) -> std::result::Result<Self::Value, E> {
        self.v.visit_u8(x)
    }
    fn visit_u16<E: de::Error>(self, x: u16) -> std::result::Result<Self::Value, E> {
        self.v.visit_u16(x)
    }
    fn visit_u32<E: de::Error>(self, x: u32) -> std::result::Result<Self::Value, E> {
        self.v.visit_u32(x)
    }
    fn visit_u64<E: de::Error>(self, x: u64) -> std::result::Result<Self::Value, E> {
        self.v.visit_u64(x)
    }
    fn visit_u128<E: de::Error>(self, x: u128) -> std::result::Result<Self::Value, E> {
        self.v.visit_u128(x)
    }
    fn visit_f32<E: de::Error>(self, x: f32) -> std::result::Result<Self::Value, E> {
        self.v.visit_f32(x)
    }
    fn visit_f64<E: de::Error>(self, x: f64) -> std::result::Result<Self::Value, E> {
        self.v.visit_f64(x)
    }
    fn visit_char<E: de::Error>(self, x: char) -> std::result::Result<Self::Value, E> {
        self.v.visit_char(x)
    }
    fn visit_str<E: de::Error>(self, x: &str) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.len().saturating_mul(2))?;
        self.v.visit_str(x)
    }
    fn visit_borrowed_str<E: de::Error>(self, x: &'de str) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.len().saturating_mul(2))?;
        self.v.visit_borrowed_str(x)
    }
    fn visit_string<E: de::Error>(self, x: String) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.capacity())?;
        self.v.visit_string(x)
    }
    fn visit_bytes<E: de::Error>(self, x: &[u8]) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.len().saturating_mul(2))?;
        self.v.visit_bytes(x)
    }
    fn visit_borrowed_bytes<E: de::Error>(
        self,
        x: &'de [u8],
    ) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.len().saturating_mul(2))?;
        self.v.visit_borrowed_bytes(x)
    }
    fn visit_byte_buf<E: de::Error>(self, x: Vec<u8>) -> std::result::Result<Self::Value, E> {
        self.b.borrow_mut().charge(x.capacity())?;
        self.v.visit_byte_buf(x)
    }
    fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
        self.v.visit_unit()
    }
    fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
        self.v.visit_none()
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> std::result::Result<Self::Value, D::Error> {
        self.v.visit_some(B { d, b: self.b })
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(
        self,
        d: D,
    ) -> std::result::Result<Self::Value, D::Error> {
        self.b
            .borrow_mut()
            .charge(std::mem::size_of::<T::Value>().saturating_mul(2))?;
        self.v.visit_newtype_struct(B { d, b: self.b })
    }
    fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> std::result::Result<Self::Value, A::Error> {
        self.v.visit_seq(B { d: a, b: self.b })
    }
    fn visit_map<A: MapAccess<'de>>(self, a: A) -> std::result::Result<Self::Value, A::Error> {
        self.v.visit_map(B { d: a, b: self.b })
    }
    fn visit_enum<A: EnumAccess<'de>>(self, a: A) -> std::result::Result<Self::Value, A::Error> {
        self.v.visit_enum(B { d: a, b: self.b })
    }
}
impl<'de, A: SeqAccess<'de>> SeqAccess<'de> for B<'_, A> {
    type Error = A::Error;
    fn next_element_seed<S: DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<Option<S::Value>, Self::Error> {
        self.b.borrow_mut().charge(
            std::mem::size_of::<S::Value>()
                .saturating_mul(4)
                .saturating_add(128),
        )?;
        self.d.next_element_seed(Seed { s, b: self.b })
    }
    fn size_hint(&self) -> Option<usize> {
        Some(0)
    }
}
impl<'de, A: MapAccess<'de>> MapAccess<'de> for B<'_, A> {
    type Error = A::Error;
    fn next_key_seed<S: DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<Option<S::Value>, Self::Error> {
        self.b.borrow_mut().charge(
            std::mem::size_of::<S::Value>()
                .saturating_mul(4)
                .saturating_add(128),
        )?;
        self.d.next_key_seed(Seed { s, b: self.b })
    }
    fn next_value_seed<S: DeserializeSeed<'de>>(
        &mut self,
        s: S,
    ) -> std::result::Result<S::Value, Self::Error> {
        self.b.borrow_mut().charge(
            std::mem::size_of::<S::Value>()
                .saturating_mul(4)
                .saturating_add(128),
        )?;
        self.d.next_value_seed(Seed { s, b: self.b })
    }
    fn size_hint(&self) -> Option<usize> {
        Some(0)
    }
}
impl<'a, 'de, A: EnumAccess<'de>> EnumAccess<'de> for B<'a, A> {
    type Error = A::Error;
    type Variant = B<'a, A::Variant>;
    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        s: S,
    ) -> std::result::Result<(S::Value, Self::Variant), Self::Error> {
        let (x, d) = self.d.variant_seed(Seed { s, b: self.b })?;
        Ok((x, B { d, b: self.b }))
    }
}
impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for B<'_, A> {
    type Error = A::Error;
    fn unit_variant(self) -> std::result::Result<(), Self::Error> {
        self.d.unit_variant()
    }
    fn newtype_variant_seed<S: DeserializeSeed<'de>>(
        self,
        s: S,
    ) -> std::result::Result<S::Value, Self::Error> {
        self.b.borrow_mut().charge(
            std::mem::size_of::<S::Value>()
                .saturating_mul(4)
                .saturating_add(128),
        )?;
        self.d.newtype_variant_seed(Seed { s, b: self.b })
    }
    fn tuple_variant<T: Visitor<'de>>(
        self,
        len: usize,
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        self.d.tuple_variant(len, V { v, b: self.b })
    }
    fn struct_variant<T: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        v: T,
    ) -> std::result::Result<T::Value, Self::Error> {
        self.d.struct_variant(fields, V { v, b: self.b })
    }
}
pub(super) fn world(bytes: &[u8]) -> Result<crate::save::WorldSave> {
    if bytes.len() > WORLD_BYTES {
        return Err(Error::Limit("world.dat bytes"));
    }
    let (payload, version) = crate::save_format::split_save_footer(bytes);
    if version != Some(crate::save_format::SAVE_FORMAT_VERSION) {
        return Err(Error::Invalid("exact current save footer"));
    }
    let value: crate::save::WorldSave = binary(payload, RESIDENT - bytes.len())?;
    super::validate_neutral(&value)?;
    Ok(value)
}

pub(super) fn metadata(bytes: &[u8]) -> Result<crate::save::WorldMeta> {
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(Error::Limit("metadata bytes"));
    }
    let b = RefCell::new(Budget {
        // Reserve input capacity and serde_json's one scratch buffer before
        // permitting owned field allocations; each can grow to twice length.
        remaining: 16 * 1024 * 1024 - bytes.len() * 4,
        depth: 0,
    });
    let mut d = serde_json::Deserializer::from_slice(bytes);
    let value = crate::save::WorldMeta::deserialize(B { d: &mut d, b: &b })
        .map_err(|_| Error::Invalid("bounded metadata"))?;
    d.end()
        .map_err(|_| Error::Invalid("metadata trailing bytes"))?;
    // Exact sender codec: unknown fields/default recovery are not authorised.
    let mut check = Compare {
        expected: bytes,
        offset: 0,
    };
    serde_json::to_writer(&mut check, &value)
        .map_err(|_| Error::Invalid("metadata canonical schema"))?;
    if check.offset != bytes.len() {
        return Err(Error::Invalid("metadata canonical length"));
    }
    Ok(value)
}

pub(super) fn binary<T: serde::de::DeserializeOwned + serde::Serialize>(
    payload: &[u8],
    resident: usize,
) -> Result<T> {
    let options = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(payload.len() as u64)
        .reject_trailing_bytes();
    let b = RefCell::new(Budget {
        remaining: resident,
        depth: 0,
    });
    b.borrow_mut()
        .charge::<bincode::Error>(std::mem::size_of::<T>().saturating_mul(2))
        .map_err(|_| Error::Limit("decoded object"))?;
    let mut d = bincode::Deserializer::from_slice(payload, options);
    let value = T::deserialize(B { d: &mut d, b: &b })
        .map_err(|_| Error::Invalid("strict bounded payload"))?;
    if bincode::serialized_size(&value).map_err(|_| Error::Invalid("encoded size"))?
        != payload.len() as u64
    {
        return Err(Error::Invalid("payload trailing bytes"));
    }
    Ok(value)
}
pub(super) fn sidecar(bytes: &[u8]) -> Result<()> {
    if bytes.len() > 65536 {
        return Err(Error::Limit("sidecar bytes"));
    }
    use crate::joiner_save::{JoinerSave, JoinerSaveV1, JoinerSaveV2};
    let saved = match bytes.first() {
        Some(1) => JoinerSave::from_v1(binary::<JoinerSaveV1>(bytes, 2 * 1024 * 1024)?)
            .map_err(|_| Error::Invalid("sidecar v1"))?,
        Some(2) => JoinerSave::from_v2(binary::<JoinerSaveV2>(bytes, 2 * 1024 * 1024)?),
        Some(3) => binary::<JoinerSave>(bytes, 2 * 1024 * 1024)?,
        _ => return Err(Error::Invalid("sidecar version")),
    };
    JoinerSave::decode(bytes).map_err(|_| Error::Invalid("sidecar validation"))?;
    saved
        .window()
        .map_err(|_| Error::Invalid("actual joiner sidecar v1"))?;
    Ok(())
}

struct Compare<'a> {
    expected: &'a [u8],
    offset: usize,
}
impl std::io::Write for Compare<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let end = self
            .offset
            .checked_add(b.len())
            .ok_or_else(|| std::io::Error::other("length"))?;
        if self.expected.get(self.offset..end) != Some(b) {
            return Err(std::io::Error::other("noncanonical"));
        }
        self.offset = end;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
