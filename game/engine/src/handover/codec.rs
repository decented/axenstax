use super::{Error, Result};

pub(crate) fn frame(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8], limit: usize) -> Result<Self> {
        if bytes.len() > limit {
            return Err(Error::Limit("binary record"));
        }
        Ok(Self { bytes, at: 0 })
    }
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(len).ok_or(Error::Limit("offset"))?;
        let bytes = self
            .bytes
            .get(self.at..end)
            .ok_or(Error::Invalid("truncated record"))?;
        self.at = end;
        Ok(bytes)
    }
    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| Error::Invalid("fixed field"))
    }
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
    pub(crate) fn framed(&mut self, max: usize) -> Result<&'a [u8]> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(Error::Limit("framed field"));
        }
        self.take(len)
    }
    pub(crate) fn label(&mut self, label: &str) -> Result<()> {
        if self.framed(label.len())? != label.as_bytes() {
            return Err(Error::Invalid("domain"));
        }
        Ok(())
    }
    pub(crate) fn end(self) -> Result<()> {
        if self.at != self.bytes.len() {
            return Err(Error::Invalid("trailing bytes"));
        }
        Ok(())
    }
}
