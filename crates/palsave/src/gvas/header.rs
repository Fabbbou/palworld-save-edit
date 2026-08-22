//! GVAS header. Layout ported from uesave-rs (trumank/uesave-rs, MIT) `Header::read`/
//! `write`, which is the more rigorous of the two reference implementations named in
//! the project plan for exactly this reason.

use super::error::GvasError;
use super::primitives::{
    FString, Guid, read_fstring, read_guid, read_i32_le, read_u16_le, read_u32_le, write_fstring,
    write_guid, write_i32_le, write_u16_le, write_u32_le,
};

pub const GVAS_MAGIC: u32 = u32::from_le_bytes(*b"GVAS");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFormatEntry {
    pub id: Guid,
    pub value: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub magic: u32,
    pub save_game_version: u32,
    pub package_version_ue4: u32,
    /// Present when save_game_version >= 3 (and != 34, a game-specific outlier
    /// uesave-rs also carves out — kept here to match its read exactly).
    pub package_version_ue5: Option<u32>,
    pub engine_version_major: u16,
    pub engine_version_minor: u16,
    pub engine_version_patch: u16,
    pub engine_version_build: u32,
    pub engine_version_branch: FString,
    /// Present when engine version >= 4.12.
    pub custom_version: Option<(u32, Vec<CustomFormatEntry>)>,
}

impl Header {
    /// True when this header uses the complete-type-name property tag format
    /// (engine >= 5.4). `crates/palsave/src/gvas/mod.rs` refuses to parse properties
    /// under this format — see `GvasError::UnsupportedPropertyTagFormat`.
    pub fn uses_new_property_tag_format(&self) -> bool {
        (self.engine_version_major, self.engine_version_minor) >= (5, 4)
    }

    pub fn has_property_guid(&self) -> bool {
        (self.engine_version_major, self.engine_version_minor) >= (4, 12)
    }

    pub fn read(buf: &[u8], pos: &mut usize) -> Result<Header, GvasError> {
        let magic = read_u32_le(buf, pos)?;
        let save_game_version = read_u32_le(buf, pos)?;
        let package_version_ue4 = read_u32_le(buf, pos)?;
        let package_version_ue5 = if save_game_version >= 3 && save_game_version != 34 {
            Some(read_u32_le(buf, pos)?)
        } else {
            None
        };
        let engine_version_major = read_u16_le(buf, pos)?;
        let engine_version_minor = read_u16_le(buf, pos)?;
        let engine_version_patch = read_u16_le(buf, pos)?;
        let engine_version_build = read_u32_le(buf, pos)?;
        let engine_version_branch = read_fstring(buf, pos)?;

        let custom_version = if (engine_version_major, engine_version_minor) >= (4, 12) {
            let format_version = read_u32_le(buf, pos)?;
            let count = read_u32_le(buf, pos)?;
            let mut entries = Vec::with_capacity(count as usize);
            for _ in 0..count {
                entries.push(CustomFormatEntry {
                    id: read_guid(buf, pos)?,
                    value: read_i32_le(buf, pos)?,
                });
            }
            Some((format_version, entries))
        } else {
            None
        };

        Ok(Header {
            magic,
            save_game_version,
            package_version_ue4,
            package_version_ue5,
            engine_version_major,
            engine_version_minor,
            engine_version_patch,
            engine_version_build,
            engine_version_branch,
            custom_version,
        })
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        write_u32_le(out, self.magic);
        write_u32_le(out, self.save_game_version);
        write_u32_le(out, self.package_version_ue4);
        if let Some(ue5) = self.package_version_ue5 {
            write_u32_le(out, ue5);
        }
        write_u16_le(out, self.engine_version_major);
        write_u16_le(out, self.engine_version_minor);
        write_u16_le(out, self.engine_version_patch);
        write_u32_le(out, self.engine_version_build);
        write_fstring(out, &self.engine_version_branch);
        if let Some((format_version, entries)) = &self.custom_version {
            write_u32_le(out, *format_version);
            write_u32_le(out, entries.len() as u32);
            for e in entries {
                write_guid(out, &e.id);
                write_i32_le(out, e.value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gvas::primitives::write_i32_le as wi32;

    fn synthetic_header_bytes(engine_major: u16, engine_minor: u16) -> Vec<u8> {
        let mut buf = Vec::new();
        write_u32_le(&mut buf, GVAS_MAGIC);
        write_u32_le(&mut buf, 3); // save_game_version, triggers ue5 field
        write_u32_le(&mut buf, 522); // package_version_ue4
        write_u32_le(&mut buf, 1007); // package_version_ue5
        write_u16_le(&mut buf, engine_major);
        write_u16_le(&mut buf, engine_minor);
        write_u16_le(&mut buf, 1); // patch
        write_u32_le(&mut buf, 12345); // build
        wi32(&mut buf, 0); // engine_version_branch: empty fstring
        if (engine_major, engine_minor) >= (4, 12) {
            write_u32_le(&mut buf, 1); // custom version format
            write_u32_le(&mut buf, 1); // one entry
            write_guid(&mut buf, &[7u8; 16]);
            write_i32_le(&mut buf, 42);
        }
        buf
    }

    #[test]
    fn header_round_trips() {
        let bytes = synthetic_header_bytes(5, 1);
        let mut pos = 0;
        let header = Header::read(&bytes, &mut pos).expect("read");
        assert_eq!(pos, bytes.len());
        assert_eq!(header.magic, GVAS_MAGIC);
        assert!(!header.uses_new_property_tag_format());
        assert!(header.has_property_guid());

        let mut out = Vec::new();
        header.write(&mut out);
        assert_eq!(out, bytes);
    }

    #[test]
    fn pre_412_header_has_no_custom_version() {
        let bytes = synthetic_header_bytes(4, 0);
        let mut pos = 0;
        let header = Header::read(&bytes, &mut pos).expect("read");
        assert_eq!(pos, bytes.len());
        assert!(header.custom_version.is_none());

        let mut out = Vec::new();
        header.write(&mut out);
        assert_eq!(out, bytes);
    }

    #[test]
    fn engine_54_plus_flags_new_tag_format() {
        let bytes = synthetic_header_bytes(5, 4);
        let mut pos = 0;
        let header = Header::read(&bytes, &mut pos).expect("read");
        assert!(header.uses_new_property_tag_format());
    }
}
