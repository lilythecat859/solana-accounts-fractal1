use std::io::{Cursor, Write};
use lz4::{Decoder, EncoderBuilder};

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut enc = EncoderBuilder::new().level(1).build(Vec::new()).unwrap();
    enc.write_all(data).unwrap();
    let (out, res) = enc.finish();
    res.unwrap();
    out
}

pub fn decompress(data: &[u8]) -> Vec<u8> {
    let mut dec = Decoder::new(Cursor::new(data)).unwrap();
    let mut out = Vec::new();
    std::io::copy(&mut dec, &mut out).unwrap();
    out
}
