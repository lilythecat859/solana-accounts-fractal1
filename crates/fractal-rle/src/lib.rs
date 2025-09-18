//! LZ4 compression / decompression helpers.
//! Errors are propagated instead of panicking.

use std::io::{Cursor, Write};
use lz4::{Decoder, EncoderBuilder};

/// Compress `data` with LZ4 level 4.
/// Returns the compressed bytes or the underlying `lz4::Error`.
pub fn compress(data: &[u8]) -> Result<Vec<u8>, lz4::Error> {
    let mut enc = EncoderBuilder::new()
        .level(4)
        .build(Vec::new())?;
    enc.write_all(data)?;
    let (out, _res) = enc.finish()?;
    Ok(out)
}

/// Decompress LZ4‑compressed `data`.
/// Returns the original bytes or the underlying `lz4::Error`.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, lz4::Error> {
    let mut dec = Decoder::new(Cursor::new(data))?;
    let mut out = Vec::new();
    std::io::copy(&mut dec, &mut out)?;
    Ok(out)
}
