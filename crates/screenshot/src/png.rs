//! Just enough PNG to read pixels back.
//!
//! The harness does not process images; it needs to *check* one — that the
//! frame a gate is about really shows what the parser says the app drew. So
//! this decodes 8-bit RGB/RGBA, non-interlaced, which is what `grim` writes,
//! and refuses anything else rather than guessing.

use std::io::{Error, ErrorKind, Read, Result};
use std::path::Path;

pub struct Image {
    pub width:  u32,
    pub height: u32,
    pub(crate) bpp:    usize,
    pub(crate) data:   Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        if x >= self.width || y >= self.height {
            return (0, 0, 0);
        }
        let i = (y as usize * self.width as usize + x as usize) * self.bpp;
        (self.data[i], self.data[i + 1], self.data[i + 2])
    }
}

/// Width and height from the IHDR alone — cheap, and enough to enforce the
/// capture invariant without decompressing anything.
pub fn size(path: &Path) -> Result<(u32, u32)> {
    let bytes = std::fs::read(path)?;
    header(&bytes).map(|h| (h.0, h.1))
}

fn header(bytes: &[u8]) -> Result<(u32, u32, u8, u8, u8)> {
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err(Error::new(ErrorKind::InvalidData, "not a PNG"));
    }
    let n = |o: usize| u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
    Ok((n(16), n(20), bytes[24], bytes[25], bytes[28]))
}

pub fn decode(path: &Path) -> Result<Image> {
    let bytes = std::fs::read(path)?;
    let (width, height, depth, color, interlace) = header(&bytes)?;
    if depth != 8 || interlace != 0 || !(color == 2 || color == 6) {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("unsupported PNG (depth {depth}, colour type {color}, interlace {interlace})"),
        ));
    }
    let bpp = if color == 6 { 4 } else { 3 };

    let mut idat = Vec::new();
    let mut off = 8;
    while off + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]) as usize;
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

    let stride = width as usize * bpp;
    let mut data = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let line = y * (stride + 1);
        // `line + 1 + stride` is the end of this scanline: one filter byte
        // then the row. The first spelling of this allowed `line + stride ==
        // raw.len()`, which slices one past the end — a PNG truncated by
        // exactly one byte panicked instead of erroring.
        if line + 1 + stride > raw.len() {
            return Err(Error::new(ErrorKind::InvalidData, "truncated PNG data"));
        }
        let filter = raw[line];
        let src = &raw[line + 1..line + 1 + stride];
        for x in 0..stride {
            let a = if x >= bpp { data[y * stride + x - bpp] } else { 0 };
            let b = if y > 0 { data[(y - 1) * stride + x] } else { 0 };
            let c = if x >= bpp && y > 0 { data[(y - 1) * stride + x - bpp] } else { 0 };
            let v = src[x];
            data[y * stride + x] = match filter {
                0 => v,
                1 => v.wrapping_add(a),
                2 => v.wrapping_add(b),
                3 => v.wrapping_add((((a as u16) + (b as u16)) / 2) as u8),
                4 => v.wrapping_add(paeth(a, b, c)),
                other => return Err(Error::new(ErrorKind::InvalidData, format!("unknown PNG filter {other}"))),
            };
        }
    }
    Ok(Image { width, height, bpp, data })
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Write an RGB image back out as a PNG.
///
/// Needed for one thing only: drawing a gate's cell coordinates onto the frame
/// a human opens. A violation the harness has already located should not send
/// anybody counting cells across a screenshot.
pub fn encode(path: &Path, width: u32, height: u32, rgb: &[u8]) -> Result<()> {
    use std::io::Write;

    let mut raw = Vec::with_capacity(rgb.len() + height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter: none. The frames are flat colour; filtering buys little.
        raw.extend_from_slice(&rgb[y * width as usize * 3..(y + 1) * width as usize * 3]);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw)?;
    let idat = encoder.finish()?;

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    std::fs::write(path, out)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let mut crc = crc32(kind);
    crc = crc32_continue(crc, body);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// PNG's CRC-32 over one chunk. `crc32_continue` finalises with the standard
/// inversion, so a fresh run starts from the finalised value of nothing — 0.
fn crc32(bytes: &[u8]) -> u32 {
    crc32_continue(0, bytes)
}

fn crc32_continue(previous: u32, bytes: &[u8]) -> u32 {
    let mut crc = previous ^ 0xffff_ffff;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    crc ^ 0xffff_ffff
}

impl Image {
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

/// Outline each cell a gate reported, on a copy of the frame.
///
/// Outline rather than fill, so the marker never hides the thing it points at,
/// and in a colour that cannot be in the app: every value in the frame comes
/// from the design palette, so magenta can only be an annotation. The existing
/// design-iteration harness paints unset cells magenta for the same reason.
pub fn annotate(source: &Path, destination: &Path, cells: &[(u16, u16)], cell_w: u32, cell_h: u32) -> Result<()> {
    const MARK: [u8; 3] = [0xff, 0x00, 0xff];

    let (width, height, mut rgb) = decode(source)?.into_rgb();
    let mut put = |x: u32, y: u32| {
        if x < width && y < height {
            let i = (y as usize * width as usize + x as usize) * 3;
            rgb[i..i + 3].copy_from_slice(&MARK);
        }
    };

    for (row, col) in cells {
        let (x0, y0) = (*col as u32 * cell_w, *row as u32 * cell_h);
        for x in x0..(x0 + cell_w).min(width) {
            put(x, y0);
            put(x, (y0 + cell_h).saturating_sub(1));
        }
        for y in y0..(y0 + cell_h).min(height) {
            put(x0, y);
            put((x0 + cell_w).saturating_sub(1), y);
        }
    }
    encode(destination, width, height, &rgb)
}
