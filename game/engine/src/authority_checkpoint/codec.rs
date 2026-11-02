//! The checkpoint's byte form: fixed-width little-endian integers, `u32` length
//! prefixes, `f32`/`f64` as their bits with `-0.0` folded onto `0.0`. Nothing
//! here reads a clock, a hash container or the process.

use super::MAX_CHECKPOINT_BYTES;

/// `-0.0` folded onto `0.0`, so a sign-only difference in a zero is never a
/// different state. NaN payloads are left as they are.
pub(super) fn f32_bits(v: f32) -> u32 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

fn f64_bits(v: f64) -> u64 {
    if v == 0.0 { 0 } else { v.to_bits() }
}

/// Appends canonical bytes, up to the hard cap. Past the cap it keeps counting
/// and stops storing, so a world far over it costs no memory on its way to the
/// error: an oversize checkpoint is refused whole, never cut short.
pub(super) struct Writer {
    buf: Vec<u8>,
    len: usize,
}

impl Writer {
    pub(super) fn new() -> Self {
        Self { buf: Vec::new(), len: 0 }
    }

    /// Bytes written so far, including any past the cap.
    pub(super) fn len(&self) -> usize {
        self.len
    }

    pub(super) fn over_cap(&self) -> bool {
        self.len > MAX_CHECKPOINT_BYTES
    }

    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.len += bytes.len();
        if self.len <= MAX_CHECKPOINT_BYTES {
            self.buf.extend_from_slice(bytes);
        }
    }

    pub(super) fn u8(&mut self, v: u8) {
        self.raw(&[v]);
    }
    pub(super) fn bool(&mut self, v: bool) {
        self.u8(v as u8);
    }
    pub(super) fn u16(&mut self, v: u16) {
        self.raw(&v.to_le_bytes());
    }
    pub(super) fn u32(&mut self, v: u32) {
        self.raw(&v.to_le_bytes());
    }
    pub(super) fn u64(&mut self, v: u64) {
        self.raw(&v.to_le_bytes());
    }
    pub(super) fn i32(&mut self, v: i32) {
        self.raw(&v.to_le_bytes());
    }
    pub(super) fn f32(&mut self, v: f32) {
        self.u32(f32_bits(v));
    }
    pub(super) fn f64(&mut self, v: f64) {
        self.u64(f64_bits(v));
    }
    pub(super) fn vec3(&mut self, v: glam::Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    pub(super) fn pos(&mut self, p: (i32, i32, i32)) {
        self.i32(p.0);
        self.i32(p.1);
        self.i32(p.2);
    }

    /// A collection's length. Saturates at `u32::MAX`, which is already far
    /// past the cap, so the error comes from the cap rather than a wrapped count.
    pub(super) fn count(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }

    /// A `u32` length, then the bytes.
    pub(super) fn blob(&mut self, bytes: &[u8]) {
        self.count(bytes.len());
        self.raw(bytes);
    }

    pub(super) fn str(&mut self, s: &str) {
        self.blob(s.as_bytes());
    }

    pub(super) fn opt<T>(&mut self, v: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match v {
            Some(v) => {
                self.u8(1);
                write(self, v);
            }
            None => self.u8(0),
        }
    }

    /// The stored bytes, or how many were asked for if that passed the cap.
    pub(super) fn finish(self) -> Result<Vec<u8>, usize> {
        if self.over_cap() { Err(self.len) } else { Ok(self.buf) }
    }
}

/// Why a checkpoint didn't decode. Every one of these is a bad checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Malformed(pub &'static str);

/// Reads canonical bytes, strictly: a short read, a stray flag or a length
/// past what is left is [`Malformed`], and nothing is allocated up front beyond
/// what the remaining bytes could really hold.
pub(super) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], Malformed> {
        if n > self.bytes.len() {
            return Err(Malformed("the bytes end early"));
        }
        let (head, tail) = self.bytes.split_at(n);
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Malformed> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(super) fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.array::<1>()?[0])
    }
    pub(super) fn bool(&mut self) -> Result<bool, Malformed> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Malformed("a flag that is neither 0 nor 1")),
        }
    }
    pub(super) fn u16(&mut self) -> Result<u16, Malformed> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    pub(super) fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    pub(super) fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    pub(super) fn i32(&mut self) -> Result<i32, Malformed> {
        Ok(i32::from_le_bytes(self.array()?))
    }
    pub(super) fn f32(&mut self) -> Result<f32, Malformed> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub(super) fn f64(&mut self) -> Result<f64, Malformed> {
        Ok(f64::from_bits(self.u64()?))
    }
    pub(super) fn vec3(&mut self) -> Result<glam::Vec3, Malformed> {
        Ok(glam::Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    pub(super) fn pos(&mut self) -> Result<(i32, i32, i32), Malformed> {
        Ok((self.i32()?, self.i32()?, self.i32()?))
    }
    pub(super) fn bytes32(&mut self) -> Result<[u8; 32], Malformed> {
        self.array()
    }
    pub(super) fn bytes16(&mut self) -> Result<[u8; 16], Malformed> {
        self.array()
    }

    /// A collection's length, refused if `each` bytes apiece could not fit in
    /// what is left, so a hostile count can't ask for a huge allocation.
    pub(super) fn count(&mut self, each: usize) -> Result<usize, Malformed> {
        let n = self.u32()? as usize;
        if n.saturating_mul(each.max(1)) > self.bytes.len() {
            return Err(Malformed("a length past the end of the bytes"));
        }
        Ok(n)
    }

    pub(super) fn blob(&mut self) -> Result<&'a [u8], Malformed> {
        let n = self.count(1)?;
        self.take(n)
    }

    pub(super) fn string(&mut self) -> Result<String, Malformed> {
        String::from_utf8(self.blob()?.to_vec()).map_err(|_| Malformed("text that isn't UTF-8"))
    }

    pub(super) fn opt<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, Malformed>) -> Result<Option<T>, Malformed> {
        if self.bool()? { read(self).map(Some) } else { Ok(None) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_little_endian_and_blobs_carry_a_u32_length() {
        let mut w = Writer::new();
        w.u16(0x0102);
        w.u32(0x0304_0506);
        w.u64(7);
        w.i32(-1);
        w.blob(b"ab");
        let bytes = w.finish().unwrap();
        assert_eq!(
            bytes,
            [
                0x02, 0x01, 0x06, 0x05, 0x04, 0x03, 7, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 2, 0, 0, 0, b'a', b'b'
            ]
        );
    }

    #[test]
    fn minus_zero_is_written_as_zero() {
        let mut a = Writer::new();
        a.f32(-0.0);
        a.f64(-0.0);
        let mut b = Writer::new();
        b.f32(0.0);
        b.f64(0.0);
        assert_eq!(a.finish().unwrap(), b.finish().unwrap());
        let mut w = Writer::new();
        w.f32(-1.5);
        assert_eq!(w.finish().unwrap(), (-1.5f32).to_bits().to_le_bytes());
    }

    #[test]
    fn past_the_cap_it_counts_and_is_refused_not_truncated() {
        let mut w = Writer::new();
        w.raw(&vec![1; MAX_CHECKPOINT_BYTES]);
        assert!(!w.over_cap());
        w.u8(2);
        assert!(w.over_cap());
        w.raw(&vec![3; 1000]);
        assert_eq!(w.finish(), Err(MAX_CHECKPOINT_BYTES + 1001));
    }

    #[test]
    fn exactly_the_cap_is_allowed() {
        let mut w = Writer::new();
        w.raw(&vec![1; MAX_CHECKPOINT_BYTES]);
        assert_eq!(w.finish().map(|b| b.len()), Ok(MAX_CHECKPOINT_BYTES));
    }

    #[test]
    fn a_short_read_a_stray_flag_and_a_huge_count_are_malformed() {
        assert!(Reader::new(&[1, 2]).u32().is_err());
        assert!(Reader::new(&[2]).bool().is_err());
        assert!(Reader::new(&u32::MAX.to_le_bytes()).count(1).is_err());
        assert!(Reader::new(&[1, 0, 0, 0]).blob().is_err());
        assert!(Reader::new(&[2, 0, 0, 0, 0xff, 0xff]).string().is_err());
    }

    #[test]
    fn what_is_written_reads_back() {
        let mut w = Writer::new();
        w.opt(Some(5u64), |w, v| w.u64(v));
        w.opt(None::<u64>, |w, v| w.u64(v));
        w.str("hello");
        w.pos((-1, 2, -3));
        w.vec3(glam::Vec3::new(1.0, 2.0, 3.0));
        let bytes = w.finish().unwrap();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.opt(|r| r.u64()), Ok(Some(5)));
        assert_eq!(r.opt(|r| r.u64()), Ok(None));
        assert_eq!(r.string().as_deref(), Ok("hello"));
        assert_eq!(r.pos(), Ok((-1, 2, -3)));
        assert_eq!(r.vec3(), Ok(glam::Vec3::new(1.0, 2.0, 3.0)));
        assert!(r.is_empty());
    }
}
