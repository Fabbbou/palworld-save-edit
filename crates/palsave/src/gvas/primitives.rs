//! Byte-cursor primitives for the GVAS wire format. All reads borrow from a single
//! `&[u8]`; nothing here allocates more than the small values it decodes (fstrings,
//! guids). Bulk data stays as spans, tracked by the caller.

use super::error::GvasError;

pub type Guid = [u8; 16];

fn need(buf: &[u8], pos: usize, len: usize) -> Result<(), GvasError> {
    if buf.len() < pos + len {
        Err(GvasError::UnexpectedEof {
            need: len,
            at: pos,
            have: buf.len().saturating_sub(pos),
        })
    } else {
        Ok(())
    }
}

pub fn read_u8(buf: &[u8], pos: &mut usize) -> Result<u8, GvasError> {
    need(buf, *pos, 1)?;
    let v = buf[*pos];
    *pos += 1;
    Ok(v)
}

pub fn read_u16_le(buf: &[u8], pos: &mut usize) -> Result<u16, GvasError> {
    need(buf, *pos, 2)?;
    let v = u16::from_le_bytes(buf[*pos..*pos + 2].try_into().unwrap());
    *pos += 2;
    Ok(v)
}

pub fn read_u32_le(buf: &[u8], pos: &mut usize) -> Result<u32, GvasError> {
    need(buf, *pos, 4)?;
    let v = u32::from_le_bytes(buf[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}

pub fn read_i32_le(buf: &[u8], pos: &mut usize) -> Result<i32, GvasError> {
    Ok(read_u32_le(buf, pos)? as i32)
}

pub fn read_u64_le(buf: &[u8], pos: &mut usize) -> Result<u64, GvasError> {
    need(buf, *pos, 8)?;
    let v = u64::from_le_bytes(buf[*pos..*pos + 8].try_into().unwrap());
    *pos += 8;
    Ok(v)
}

pub fn read_i64_le(buf: &[u8], pos: &mut usize) -> Result<i64, GvasError> {
    Ok(read_u64_le(buf, pos)? as i64)
}

pub fn read_f32_le(buf: &[u8], pos: &mut usize) -> Result<f32, GvasError> {
    Ok(f32::from_bits(read_u32_le(buf, pos)?))
}

pub fn read_f64_le(buf: &[u8], pos: &mut usize) -> Result<f64, GvasError> {
    Ok(f64::from_bits(read_u64_le(buf, pos)?))
}

pub fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

pub fn write_u16_le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn write_u32_le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn write_i32_le(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn write_u64_le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn write_i64_le(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn read_guid(buf: &[u8], pos: &mut usize) -> Result<Guid, GvasError> {
    need(buf, *pos, 16)?;
    let mut g = [0u8; 16];
    g.copy_from_slice(&buf[*pos..*pos + 16]);
    *pos += 16;
    Ok(g)
}

pub fn write_guid(out: &mut Vec<u8>, g: &Guid) {
    out.extend_from_slice(g);
}

pub fn read_optional_guid(buf: &[u8], pos: &mut usize) -> Result<Option<Guid>, GvasError> {
    if read_u8(buf, pos)? > 0 {
        Ok(Some(read_guid(buf, pos)?))
    } else {
        Ok(None)
    }
}

pub fn write_optional_guid(out: &mut Vec<u8>, g: &Option<Guid>) {
    match g {
        Some(g) => {
            write_u8(out, 1);
            write_guid(out, g);
        }
        None => write_u8(out, 0),
    }
}

/// An Unreal FString. Content is kept as raw code units (bytes for the ASCII/Latin1
/// form, u16 for the UTF-16LE form), split at the first NUL terminator found within
/// the declared length. Anything after that NUL — including the terminator itself, or
/// stray bytes past it — is kept verbatim in `trailing` rather than reconstructed, so
/// a round-trip write reproduces the exact original bytes even for malformed input.
/// This mirrors uesave-rs's `read_string_trailing`, which is the only lossless option;
/// its plain `read_string` silently drops anything after the first NUL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FString {
    Empty,
    Ascii {
        content: Vec<u8>,
        trailing: Vec<u8>,
    },
    Utf16 {
        content: Vec<u16>,
        trailing: Vec<u8>,
    },
}

impl FString {
    /// Best-effort borrowed &str view, for matching known ASCII identifiers
    /// (property names, type names). Returns None for anything non-ASCII/empty-ish
    /// that isn't a plain identifier — callers must not rely on this for byte fidelity.
    pub fn ascii_str(&self) -> Option<&str> {
        match self {
            FString::Ascii { content, .. } => std::str::from_utf8(content).ok(),
            _ => None,
        }
    }

    pub fn display_lossy(&self) -> String {
        match self {
            FString::Empty => String::new(),
            FString::Ascii { content, .. } => String::from_utf8_lossy(content).into_owned(),
            FString::Utf16 { content, .. } => String::from_utf16_lossy(content),
        }
    }
}

pub fn read_fstring(buf: &[u8], pos: &mut usize) -> Result<FString, GvasError> {
    let len = read_i32_le(buf, pos)?;
    if len == 0 {
        return Ok(FString::Empty);
    }
    if len > 0 {
        let total = len as usize;
        need(buf, *pos, total)?;
        let start = *pos;
        let mut content = Vec::new();
        let mut trailing = Vec::new();
        let mut read = 0usize;
        while read < total {
            let b = buf[start + read];
            read += 1;
            if b == 0 {
                trailing.push(b);
                break;
            }
            content.push(b);
        }
        while read < total {
            trailing.push(buf[start + read]);
            read += 1;
        }
        *pos = start + total;
        Ok(FString::Ascii { content, trailing })
    } else {
        let units = (-len) as usize;
        let total_bytes = units * 2;
        need(buf, *pos, total_bytes)?;
        let start = *pos;
        let mut content = Vec::new();
        let mut trailing = Vec::new();
        let mut read = 0usize;
        while read < total_bytes {
            let unit = u16::from_le_bytes([buf[start + read], buf[start + read + 1]]);
            read += 2;
            if unit == 0 {
                trailing.extend_from_slice(&unit.to_le_bytes());
                break;
            }
            content.push(unit);
        }
        while read < total_bytes {
            trailing.push(buf[start + read]);
            read += 1;
        }
        *pos = start + total_bytes;
        Ok(FString::Utf16 { content, trailing })
    }
}

pub fn write_fstring(out: &mut Vec<u8>, s: &FString) {
    match s {
        FString::Empty => write_i32_le(out, 0),
        FString::Ascii { content, trailing } => {
            write_i32_le(out, (content.len() + trailing.len()) as i32);
            out.extend_from_slice(content);
            out.extend_from_slice(trailing);
        }
        FString::Utf16 { content, trailing } => {
            let total_bytes = content.len() * 2 + trailing.len();
            write_i32_le(out, -((total_bytes / 2) as i32));
            for unit in content {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out.extend_from_slice(trailing);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(s: &FString) -> FString {
        let mut buf = Vec::new();
        write_fstring(&mut buf, s);
        let mut pos = 0;
        read_fstring(&buf, &mut pos).unwrap()
    }

    #[test]
    fn empty_round_trips() {
        assert_eq!(round_trip(&FString::Empty), FString::Empty);
    }

    #[test]
    fn ascii_round_trips() {
        let s = FString::Ascii {
            content: b"IntProperty".to_vec(),
            trailing: vec![0],
        };
        assert_eq!(round_trip(&s), s);
        assert_eq!(s.ascii_str(), Some("IntProperty"));
    }

    #[test]
    fn utf16_round_trips() {
        let content: Vec<u16> = "guild-\u{540d}\u{524d}".encode_utf16().collect();
        let s = FString::Utf16 {
            content,
            trailing: vec![0, 0],
        };
        assert_eq!(round_trip(&s), s);
    }

    #[test]
    fn ascii_without_terminator_round_trips() {
        // Declared length exactly matches content, no NUL ever appears.
        let s = FString::Ascii {
            content: b"abc".to_vec(),
            trailing: vec![],
        };
        assert_eq!(round_trip(&s), s);
    }

    #[test]
    fn guid_round_trips() {
        let g: Guid = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let mut buf = Vec::new();
        write_guid(&mut buf, &g);
        let mut pos = 0;
        assert_eq!(read_guid(&buf, &mut pos).unwrap(), g);
        assert_eq!(pos, 16);
    }

    #[test]
    fn optional_guid_round_trips_both_states() {
        for value in [None, Some([9u8; 16])] {
            let mut buf = Vec::new();
            write_optional_guid(&mut buf, &value);
            let mut pos = 0;
            assert_eq!(read_optional_guid(&buf, &mut pos).unwrap(), value);
        }
    }
}
