//! Fractal-RLE: ultra-fast LZ4 compression for account blobs
use std::io::{Cursor, Write};

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut enc = lz4::EncoderBuilder::new()
        .level(4)
        .build(Vec::new())
        .expect("lz4 encoder");
    enc.write_all(data).expect("write");
    let (out, res) = enc.finish();
    res.expect("lz4 finish");
    out
}

pub fn decompress(data: &[u8]) -> Vec<u8> {
    let mut dec = lz4::Decoder::new(Cursor::new(data)).expect("lz4 decoder");
    let mut out = Vec::new();
    std::io::copy(&mut dec, &mut out).expect("lz4 copy");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let original = vec![5u8; 1024];
        let compressed = compress(&original);
        let decompressed = decompress(&compressed);
        assert_eq!(original, decompressed);
    }
}
