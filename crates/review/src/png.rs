//! Just enough PNG to check a captured frame's pixels.
//!
//! Decodes only 8-bit RGB/RGBA non-interlaced (what `grim` writes) and refuses
//! anything else. The encoder is test-only.

use std::io::Read;
use std::path::Path;

use crate::{Error, Result};

/// A decoded frame: its pixels unfiltered, three or four bytes each.
#[derive(Debug)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    bpp: usize,
    data: Vec<u8>,
}

impl Image {
    /// The RGB of the pixel at `x`, `y`; black outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        if x >= self.width || y >= self.height {
            return (0, 0, 0);
        }
        let i = (y as usize * self.width as usize + x as usize) * self.bpp;
        (self.data[i], self.data[i + 1], self.data[i + 2])
    }

    /// Width, height and the pixels as packed RGB.
    #[cfg(test)]
    pub fn into_rgb(self) -> (u32, u32, Vec<u8>) {
        if self.bpp == 3 {
            return (self.width, self.height, self.data);
        }
        let mut rgb = Vec::with_capacity(self.width as usize * self.height as usize * 3);
        for pixel in self.data.chunks_exact(self.bpp) {
            rgb.extend_from_slice(&pixel[..3]);
        }
        (self.width, self.height, rgb)
    }
}

/// Width and height from the IHDR alone, without decompressing.
///
/// # Errors
///
/// When the file cannot be read or does not start with a PNG signature and
/// IHDR chunk.
pub fn size(path: &Path) -> Result<(u32, u32)> {
    let bytes = std::fs::read(path)?;
    header(&bytes).map(|h| (h.0, h.1))
}

fn header(bytes: &[u8]) -> Result<(u32, u32, u8, u8, u8)> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(Error::Png("not a PNG".into()));
    }
    let n = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    Ok((n(16), n(20), bytes[24], bytes[25], bytes[28]))
}

/// Reads and decodes an 8-bit RGB or RGBA, non-interlaced PNG.
///
/// # Errors
///
/// When the file cannot be read, is not a PNG, has another depth, colour type
/// or interlacing, or its data does not inflate, is truncated, or uses an
/// unknown filter.
pub fn decode(path: &Path) -> Result<Image> {
    let bytes = std::fs::read(path)?;
    let (width, height, depth, color, interlace) = header(&bytes)?;
    if depth != 8 || interlace != 0 || !(color == 2 || color == 6) {
        return Err(Error::Png(format!(
            "unsupported PNG (depth {depth}, colour type {color}, interlace {interlace})"
        )));
    }
    let bpp = if color == 6 { 4 } else { 3 };

    let mut idat = Vec::new();
    let mut off = 8;
    while off + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
            as usize;
        let kind = &bytes[off + 4..off + 8];
        let body = &bytes[off + 8..(off + 8 + len).min(bytes.len())];
        match kind {
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        off += 12 + len;
    }

    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&idat[..]).read_to_end(&mut raw)?;

    // A scanline is a filter byte then the row. Checked before allocating, so
    // the header's claimed size is never trusted.
    let stride = width as usize * bpp;
    if (raw.len() as u64) < (stride as u64 + 1) * height as u64 {
        return Err(Error::Png("truncated PNG data".into()));
    }
    let mut data = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let line = y * (stride + 1);
        let filter = raw[line];
        let src = &raw[line + 1..line + 1 + stride];
        for x in 0..stride {
            let a = if x >= bpp {
                data[y * stride + x - bpp]
            } else {
                0
            };
            let b = if y > 0 { data[(y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 {
                data[(y - 1) * stride + x - bpp]
            } else {
                0
            };
            let v = src[x];
            data[y * stride + x] = match filter {
                0 => v,
                1 => v.wrapping_add(a),
                2 => v.wrapping_add(b),
                3 => v.wrapping_add((((a as u16) + (b as u16)) / 2) as u8),
                4 => v.wrapping_add(paeth(a, b, c)),
                other => return Err(Error::Png(format!("unknown PNG filter {other}"))),
            };
        }
    }
    Ok(Image {
        width,
        height,
        bpp,
        data,
    })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = (
        (p - a as i16).abs(),
        (p - b as i16).abs(),
        (p - c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Writes `rgb`, packed 8-bit RGB, as a PNG at `path`.
///
/// # Errors
///
/// When `rgb` is not `width × height × 3` bytes, compression fails, or the
/// file cannot be written.
#[cfg(test)]
pub fn encode(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    use std::io::Write;

    let stride = width as usize * 3;
    if rgb.len() != stride * height as usize {
        return Err(Error::Png(
            "RGB buffer does not match the image size".into(),
        ));
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    for y in 0..height as usize {
        encoder.write_all(&[0])?; // filter: none. The frames are flat colour; filtering buys little.
        encoder.write_all(&rgb[y * stride..(y + 1) * stride])?;
    }
    let idat = encoder.finish()?;

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)?;
    Ok(())
}

#[cfg(test)]
fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = crc32(crc32(0, kind), body);
    out.extend_from_slice(&crc.to_be_bytes());
}

#[cfg(test)]
/// PNG's CRC-32, resumable: pass a previous result to continue it, `0` to
/// start.
fn crc32(previous: u32, bytes: &[u8]) -> u32 {
    let mut crc = previous ^ 0xffff_ffff;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_carries_the_crc_every_png_ends_with() {
        // IEND's CRC is a constant of the format.
        let mut out = Vec::new();
        chunk(&mut out, b"IEND", &[]);
        assert_eq!(
            out,
            [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]
        );
    }

    #[test]
    fn an_encoded_image_decodes_to_the_same_pixels() {
        let path =
            std::env::temp_dir().join(format!("aldwin-review-png-{}.png", std::process::id()));
        let rgb: Vec<u8> = (0..3 * 5 * 3).map(|i| i as u8 * 5).collect();
        encode(&path, 3, 5, &rgb).expect("encodes");
        let image = decode(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(image.expect("decodes").into_rgb(), (3, 5, rgb));
    }

    #[test]
    fn a_truncated_image_is_an_error_rather_than_a_panic() {
        let path =
            std::env::temp_dir().join(format!("aldwin-review-short-{}.png", std::process::id()));
        encode(&path, 3, 5, &[0; 45]).expect("encodes");
        // Claim one more row than the data holds.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[23] = 6;
        std::fs::write(&path, bytes).unwrap();
        let image = decode(&path);
        let _ = std::fs::remove_file(&path);
        assert!(image.is_err());
    }
}
