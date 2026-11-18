//! Byte buffer helpers shared by RakNet, the Bedrock protocol and the world format.

pub type Res<T> = Result<T, String>;

#[derive(Default, Clone)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> Self {
        W(Vec::with_capacity(256))
    }
    pub fn with_capacity(n: usize) -> Self {
        W(Vec::with_capacity(n))
    }
    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.0.push(v as u8);
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.0.extend_from_slice(v);
    }
    pub fn u16_be(&mut self, v: u16) {
        self.bytes(&v.to_be_bytes());
    }
    pub fn u16_le(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn i16_le(&mut self, v: i16) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u32_be(&mut self, v: u32) {
        self.bytes(&v.to_be_bytes());
    }
    pub fn u32_le(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn i32_le(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u64_be(&mut self, v: u64) {
        self.bytes(&v.to_be_bytes());
    }
    pub fn u64_le(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn f32_le(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn triad_le(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes()[..3]);
    }
    pub fn uvarint(&mut self, mut v: u32) {
        loop {
            if v < 0x80 {
                self.0.push(v as u8);
                return;
            }
            self.0.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }
    pub fn varint(&mut self, v: i32) {
        self.uvarint(((v << 1) ^ (v >> 31)) as u32);
    }
    pub fn uvarlong(&mut self, mut v: u64) {
        loop {
            if v < 0x80 {
                self.0.push(v as u8);
                return;
            }
            self.0.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }
    pub fn varlong(&mut self, v: i64) {
        self.uvarlong(((v << 1) ^ (v >> 63)) as u64);
    }
    /// Bedrock string: unsigned varint length + bytes.
    pub fn string(&mut self, s: &str) {
        self.uvarint(s.len() as u32);
        self.bytes(s.as_bytes());
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

pub struct R<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        R { b, p: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
    pub fn eof(&self) -> bool {
        self.p >= self.b.len()
    }
    pub fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if self.p + n > self.b.len() {
            return Err(format!("unexpected end of buffer (need {n}, have {})", self.remaining()));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.p.min(self.b.len())..];
        self.p = self.b.len();
        s
    }
    fn arr<const N: usize>(&mut self) -> Res<[u8; N]> {
        Ok(self.take(N)?.try_into().unwrap())
    }
    pub fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Res<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u16_be(&mut self) -> Res<u16> {
        Ok(u16::from_be_bytes(self.arr()?))
    }
    pub fn u16_le(&mut self) -> Res<u16> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn i16_be(&mut self) -> Res<i16> {
        Ok(i16::from_be_bytes(self.arr()?))
    }
    pub fn i16_le(&mut self) -> Res<i16> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    pub fn u32_be(&mut self) -> Res<u32> {
        Ok(u32::from_be_bytes(self.arr()?))
    }
    pub fn i32_be(&mut self) -> Res<i32> {
        Ok(i32::from_be_bytes(self.arr()?))
    }
    pub fn u32_le(&mut self) -> Res<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn i32_le(&mut self) -> Res<i32> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    pub fn u64_be(&mut self) -> Res<u64> {
        Ok(u64::from_be_bytes(self.arr()?))
    }
    pub fn i64_be(&mut self) -> Res<i64> {
        Ok(i64::from_be_bytes(self.arr()?))
    }
    pub fn i64_le(&mut self) -> Res<i64> {
        Ok(i64::from_le_bytes(self.arr()?))
    }
    pub fn f32_be(&mut self) -> Res<f32> {
        Ok(f32::from_be_bytes(self.arr()?))
    }
    pub fn f32_le(&mut self) -> Res<f32> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    pub fn f64_be(&mut self) -> Res<f64> {
        Ok(f64::from_be_bytes(self.arr()?))
    }
    pub fn f64_le(&mut self) -> Res<f64> {
        Ok(f64::from_le_bytes(self.arr()?))
    }
    pub fn triad_le(&mut self) -> Res<u32> {
        let b = self.take(3)?;
        Ok(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
    }
    pub fn uvarint(&mut self) -> Res<u32> {
        let mut v: u32 = 0;
        for i in 0..5 {
            let b = self.u8()?;
            v |= ((b & 0x7f) as u32) << (i * 7);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("varint too long".into())
    }
    pub fn varint(&mut self) -> Res<i32> {
        let v = self.uvarint()?;
        Ok((v >> 1) as i32 ^ -((v & 1) as i32))
    }
    pub fn uvarlong(&mut self) -> Res<u64> {
        let mut v: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            v |= ((b & 0x7f) as u64) << (i * 7);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err("varlong too long".into())
    }
    pub fn varlong(&mut self) -> Res<i64> {
        let v = self.uvarlong()?;
        Ok((v >> 1) as i64 ^ -((v & 1) as i64))
    }
    pub fn string(&mut self) -> Res<String> {
        let n = self.uvarint()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
}
