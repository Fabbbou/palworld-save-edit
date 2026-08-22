//! Container codec: compression only, no GVAS parsing.
//!
//! Layout of a raw header (12 bytes): uncompressed_len: u32 LE, compressed_len: u32 LE,
//! magic: [u8; 3] ("PlZ" zlib, "PlM" Oodle Mermaid, "CNK" Game Pass/WGS wrapper),
//! save_type: u8 (0x31 = one compression pass, 0x32 = two).
//!
//! A CNK-wrapped file has an outer 12-byte header (magic "CNK") immediately followed
//! by the real 12-byte header at offset 12, then the payload.

mod error;

pub use error::ContainerError;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use oozextract::Extractor;
use std::io::{Read, Write};

const HEADER_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    Zlib,
    OodleMermaid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Passes {
    One,
    Two,
}

impl Passes {
    fn from_save_type(save_type: u8) -> Result<Self, ContainerError> {
        match save_type {
            0x31 => Ok(Passes::One),
            0x32 => Ok(Passes::Two),
            other => Err(ContainerError::UnknownSaveType(other)),
        }
    }

    fn byte(self) -> u8 {
        match self {
            Passes::One => 0x31,
            Passes::Two => 0x32,
        }
    }
}

/// A decoded save container: the algorithm and pass count needed to write it back
/// in a compatible form, plus the decompressed GVAS payload.
pub struct Container {
    pub algorithm: Algorithm,
    pub passes: Passes,
    pub was_cnk_wrapped: bool,
    pub gvas: Vec<u8>,
}

struct RawHeader {
    uncompressed_len: u32,
    compressed_len: u32,
    magic: [u8; 3],
    save_type: u8,
}

fn read_raw_header(bytes: &[u8], base: usize) -> Result<RawHeader, ContainerError> {
    if bytes.len() < base + HEADER_LEN {
        return Err(ContainerError::TooShort {
            need: base + HEADER_LEN,
            have: bytes.len(),
        });
    }
    let uncompressed_len = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap());
    let compressed_len = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap());
    let magic: [u8; 3] = bytes[base + 8..base + 11].try_into().unwrap();
    let save_type = bytes[base + 11];
    Ok(RawHeader {
        uncompressed_len,
        compressed_len,
        magic,
        save_type,
    })
}

fn zlib_inflate(input: &[u8], size_hint: usize) -> Result<Vec<u8>, ContainerError> {
    let mut decoder = ZlibDecoder::new(input);
    let mut out = Vec::with_capacity(size_hint);
    decoder
        .read_to_end(&mut out)
        .map_err(ContainerError::Zlib)?;
    Ok(out)
}

fn zlib_deflate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(input)
        .expect("writing to a Vec-backed encoder never fails");
    encoder
        .finish()
        .expect("finishing a Vec-backed encoder never fails")
}

pub fn decode(bytes: &[u8]) -> Result<Container, ContainerError> {
    let outer = read_raw_header(bytes, 0)?;

    let (header, base, was_cnk_wrapped) = if &outer.magic == b"CNK" {
        (read_raw_header(bytes, HEADER_LEN)?, HEADER_LEN, true)
    } else {
        (outer, 0, false)
    };

    let payload_start = base + HEADER_LEN;
    let payload_end = payload_start + header.compressed_len as usize;
    let payload =
        bytes
            .get(payload_start..payload_end)
            .ok_or(ContainerError::PayloadOutOfBounds {
                start: payload_start,
                compressed_len: header.compressed_len,
                file_len: bytes.len(),
            })?;

    let passes = Passes::from_save_type(header.save_type)?;

    let algorithm = match &header.magic {
        b"PlZ" => Algorithm::Zlib,
        b"PlM" => Algorithm::OodleMermaid,
        other => return Err(ContainerError::UnknownMagic(*other)),
    };

    // A second pass (save_type 0x32) is always a zlib inflate over the result of the
    // first — matching PlZ's own double-deflate scheme. For PlM this combination has
    // no known real-world sample and no header field for the required intermediate
    // buffer size, so it's rejected rather than guessed. See ContainerError docs.
    if algorithm == Algorithm::OodleMermaid && passes == Passes::Two {
        return Err(ContainerError::UnsupportedOodleDoublePass);
    }

    let after_first_pass = match algorithm {
        Algorithm::Zlib => zlib_inflate(payload, header.uncompressed_len as usize)?,
        Algorithm::OodleMermaid => {
            let mut out = vec![0u8; header.uncompressed_len as usize];
            let n = Extractor::new()
                .read_from_slice(payload, &mut out)
                .map_err(ContainerError::Oodle)?;
            out.truncate(n);
            out
        }
    };

    let gvas = match passes {
        Passes::One => after_first_pass,
        Passes::Two => zlib_inflate(&after_first_pass, header.uncompressed_len as usize)?,
    };

    Ok(Container {
        algorithm,
        passes,
        was_cnk_wrapped,
        gvas,
    })
}

/// No open-source Oodle COMPRESSOR exists. A PlM container is always written back as
/// PlZ/0x32 (double zlib pass); the game reads both formats. A PlZ container stays
/// PlZ at its original pass count. CNK re-wrapping is not attempted — untested without
/// a Game Pass fixture, and the phase spec only requires unwrapping it on read.
pub fn encode(gvas: &[u8], original: &Container) -> Vec<u8> {
    let passes = match original.algorithm {
        Algorithm::OodleMermaid => Passes::Two,
        Algorithm::Zlib => original.passes,
    };

    let compressed = match passes {
        Passes::One => zlib_deflate(gvas),
        Passes::Two => zlib_deflate(&zlib_deflate(gvas)),
    };

    let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
    out.extend_from_slice(&(gvas.len() as u32).to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(b"PlZ");
    out.push(passes.byte());
    out.extend_from_slice(&compressed);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plz(gvas: &[u8], passes: Passes) -> Vec<u8> {
        let compressed = match passes {
            Passes::One => zlib_deflate(gvas),
            Passes::Two => zlib_deflate(&zlib_deflate(gvas)),
        };
        let mut out = Vec::new();
        out.extend_from_slice(&(gvas.len() as u32).to_le_bytes());
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(b"PlZ");
        out.push(passes.byte());
        out.extend_from_slice(&compressed);
        out
    }

    #[test]
    fn plz_single_pass_round_trips() {
        let gvas = b"synthetic GVAS payload, single pass".to_vec();
        let sav = make_plz(&gvas, Passes::One);

        let container = decode(&sav).expect("decode");
        assert_eq!(container.algorithm, Algorithm::Zlib);
        assert_eq!(container.passes, Passes::One);
        assert!(!container.was_cnk_wrapped);
        assert_eq!(container.gvas, gvas);

        let re_encoded = encode(&container.gvas, &container);
        let re_decoded = decode(&re_encoded).expect("re-decode");
        assert_eq!(re_decoded.gvas, gvas);
    }

    #[test]
    fn plz_double_pass_round_trips() {
        let gvas = b"synthetic GVAS payload, double deflate pass this time".to_vec();
        let sav = make_plz(&gvas, Passes::Two);

        let container = decode(&sav).expect("decode");
        assert_eq!(container.passes, Passes::Two);
        assert_eq!(container.gvas, gvas);

        let re_encoded = encode(&container.gvas, &container);
        let re_decoded = decode(&re_encoded).expect("re-decode");
        assert_eq!(re_decoded.gvas, gvas);
        assert_eq!(re_decoded.passes, Passes::Two);
    }

    #[test]
    fn cnk_wrapper_is_unwrapped_on_read() {
        let gvas = b"payload inside a Game Pass CNK wrapper".to_vec();
        let inner = make_plz(&gvas, Passes::One);

        let mut sav = Vec::new();
        // Outer CNK header: lengths are irrelevant to unwrapping, only the magic is read.
        sav.extend_from_slice(&0u32.to_le_bytes());
        sav.extend_from_slice(&0u32.to_le_bytes());
        sav.extend_from_slice(b"CNK");
        sav.push(0);
        sav.extend_from_slice(&inner);

        let container = decode(&sav).expect("decode");
        assert!(container.was_cnk_wrapped);
        assert_eq!(container.gvas, gvas);
    }

    #[test]
    fn unknown_magic_is_an_error() {
        let mut sav = Vec::new();
        sav.extend_from_slice(&4u32.to_le_bytes());
        sav.extend_from_slice(&4u32.to_le_bytes());
        sav.extend_from_slice(b"???");
        sav.push(0x31);
        sav.extend_from_slice(b"data");

        assert!(matches!(decode(&sav), Err(ContainerError::UnknownMagic(_))));
    }

    #[test]
    fn truncated_payload_is_an_error() {
        let mut sav = Vec::new();
        sav.extend_from_slice(&100u32.to_le_bytes());
        sav.extend_from_slice(&100u32.to_le_bytes()); // claims far more than we provide
        sav.extend_from_slice(b"PlZ");
        sav.push(0x31);
        sav.extend_from_slice(b"short");

        assert!(matches!(
            decode(&sav),
            Err(ContainerError::PayloadOutOfBounds { .. })
        ));
    }

    #[test]
    fn oodle_double_pass_is_refused_not_guessed() {
        let mut sav = Vec::new();
        sav.extend_from_slice(&4u32.to_le_bytes());
        sav.extend_from_slice(&4u32.to_le_bytes());
        sav.extend_from_slice(b"PlM");
        sav.push(0x32);
        sav.extend_from_slice(&[0u8; 4]);

        assert!(matches!(
            decode(&sav),
            Err(ContainerError::UnsupportedOodleDoublePass)
        ));
    }
}
