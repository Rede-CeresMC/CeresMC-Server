//! Mojang's LevelDB flavour: zlib block compression and chunk keys.

use super::*;

/// Mojang's LevelDB fork compresses blocks with zlib (id 2) or raw deflate (id 4).
pub(crate) struct ZlibCompressor {
    pub(crate) raw: bool,
}

impl rusty_leveldb::Compressor for ZlibCompressor {
    fn encode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let level = flate2::Compression::default();
        let out = if self.raw {
            let mut e = flate2::write::DeflateEncoder::new(Vec::new(), level);
            e.write_all(&block).and_then(|_| e.finish())
        } else {
            let mut e = flate2::write::ZlibEncoder::new(Vec::new(), level);
            e.write_all(&block).and_then(|_| e.finish())
        };
        out.map_err(|e| rusty_leveldb::Status::new(rusty_leveldb::StatusCode::CompressionError, &e.to_string()))
    }
    fn decode(&self, block: Vec<u8>) -> rusty_leveldb::Result<Vec<u8>> {
        let mut out = Vec::new();
        let res = if self.raw {
            flate2::read::DeflateDecoder::new(&block[..]).read_to_end(&mut out)
        } else {
            flate2::read::ZlibDecoder::new(&block[..]).read_to_end(&mut out)
        };
        res.map(|_| out)
            .map_err(|e| rusty_leveldb::Status::new(rusty_leveldb::StatusCode::CompressionError, &e.to_string()))
    }
}

pub(crate) fn bedrock_options() -> Options {
    let mut list = CompressorList::new();
    list.set_with_id(0, rusty_leveldb::compressor::NoneCompressor);
    list.set_with_id(2, ZlibCompressor { raw: false });
    list.set_with_id(4, ZlibCompressor { raw: true });
    let mut opt = Options::default();
    opt.compressor_list = Rc::new(list);
    opt.compressor = 4;
    opt.create_if_missing = true;
    opt
}

pub(crate) fn chunk_key(x: i32, z: i32, tag: u8, sub: Option<i8>) -> Vec<u8> {
    let mut k = Vec::with_capacity(10);
    k.extend_from_slice(&x.to_le_bytes());
    k.extend_from_slice(&z.to_le_bytes());
    k.push(tag);
    if let Some(y) = sub {
        k.push(y as u8);
    }
    k
}
