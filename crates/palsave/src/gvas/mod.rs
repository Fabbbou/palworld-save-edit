//! Lazy GVAS reader and byte-exact writer.
//!
//! `GvasFile::parse` walks the header and the flat top-level property list just
//! far enough to record byte spans — it never decodes an Array/Map/Struct payload.
//! `GvasFile::write` reassembles the original bytes by concatenating those spans in
//! order, so round-tripping is correct by construction: no property is ever
//! re-derived from a decoded value, only copied. Phase 4's splice engine is what
//! will replace individual spans with freshly-encoded bytes on edit.
//!
//! Only the legacy (pre-engine-5.4) property tag format is implemented — see
//! `GvasError::UnsupportedPropertyTagFormat`. No real Palworld fixture has been
//! available to confirm which format current saves actually use; ADR-001.md tracks
//! this as an open question for whoever adds the first one.

pub mod error;
pub mod header;
pub mod hints;
pub mod nav;
pub mod primitives;
pub mod property;
pub mod value;

pub use error::GvasError;
pub use header::Header;
pub use value::Value;

use primitives::read_fstring;
use property::read_property_tag;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyEntry {
    /// Lossy display name — for indexing/UI only, not used when writing.
    pub name: String,
    /// Lossy display type name — for indexing/UI only, not used when writing.
    pub type_name: String,
    /// Tag header + value bytes for this property, verbatim in the source buffer.
    pub span: Range<usize>,
}

#[derive(Debug)]
pub struct GvasFile<'a> {
    pub header: Header,
    pub save_game_type: String,
    pub properties: Vec<PropertyEntry>,
    source: &'a [u8],
    header_span: Range<usize>,
    save_game_type_span: Range<usize>,
    none_span: Range<usize>,
    trailing_span: Range<usize>,
}

impl<'a> GvasFile<'a> {
    pub fn parse(source: &'a [u8]) -> Result<Self, GvasError> {
        let mut pos = 0usize;
        let header = Header::read(source, &mut pos)?;
        let header_span = 0..pos;

        if header.uses_new_property_tag_format() {
            return Err(GvasError::UnsupportedPropertyTagFormat {
                engine_major: header.engine_version_major,
                engine_minor: header.engine_version_minor,
            });
        }

        let save_game_type_start = pos;
        let save_game_type_fs = read_fstring(source, &mut pos)?;
        let save_game_type_span = save_game_type_start..pos;
        let save_game_type = save_game_type_fs.display_lossy();

        let has_property_guid = header.has_property_guid();
        let mut properties = Vec::new();
        let none_span;
        loop {
            let entry_start = pos;
            match read_property_tag(source, &mut pos, has_property_guid)? {
                None => {
                    none_span = entry_start..pos;
                    break;
                }
                Some(tag) => {
                    let value_len = tag.size as usize;
                    if source.len() < pos + value_len {
                        return Err(GvasError::UnexpectedEof {
                            need: value_len,
                            at: pos,
                            have: source.len().saturating_sub(pos),
                        });
                    }
                    pos += value_len;
                    properties.push(PropertyEntry {
                        name: tag.name.display_lossy(),
                        type_name: tag.type_name.display_lossy(),
                        span: entry_start..pos,
                    });
                }
            }
        }
        let trailing_span = pos..source.len();

        Ok(GvasFile {
            header,
            save_game_type,
            properties,
            source,
            header_span,
            save_game_type_span,
            none_span,
            trailing_span,
        })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.source.len());
        out.extend_from_slice(&self.source[self.header_span.clone()]);
        out.extend_from_slice(&self.source[self.save_game_type_span.clone()]);
        for p in &self.properties {
            out.extend_from_slice(&self.source[p.span.clone()]);
        }
        out.extend_from_slice(&self.source[self.none_span.clone()]);
        out.extend_from_slice(&self.source[self.trailing_span.clone()]);
        out
    }

    /// Decodes one top-level property's value. Read-only — see `gvas::value` module
    /// docs for why this doesn't feed back into `write()` yet.
    pub fn materialize(&self, index: usize) -> Result<Value, GvasError> {
        value::materialize_property(
            self.source,
            &self.properties[index],
            self.header.engine_version_major,
            self.header.has_property_guid(),
            &self.properties[index].name,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use primitives::{FString, write_fstring, write_i32_le, write_u16_le, write_u32_le};
    use property::{PropertyTag, TagExtra, none_terminator, write_property_tag};

    fn ascii(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    /// Builds a minimal, self-consistent synthetic GVAS buffer: header (engine 5.1,
    /// so legacy tag format + property GUIDs both apply) + save_game_type + a few
    /// properties of different shapes + None terminator + trailing bytes.
    fn synthetic_gvas() -> Vec<u8> {
        let mut buf = Vec::new();
        write_u32_le(&mut buf, header::GVAS_MAGIC);
        write_u32_le(&mut buf, 3); // save_game_version -> ue5 field present
        write_u32_le(&mut buf, 522);
        write_u32_le(&mut buf, 1007);
        write_u16_le(&mut buf, 5); // engine major
        write_u16_le(&mut buf, 1); // engine minor (< 5.4: legacy tag format)
        write_u16_le(&mut buf, 0);
        write_u32_le(&mut buf, 0);
        write_fstring(&mut buf, &FString::Empty); // engine_version_branch
        write_u32_le(&mut buf, 1); // custom version format
        write_u32_le(&mut buf, 0); // zero custom version entries

        write_fstring(&mut buf, &ascii("PalworldSaveGame")); // save_game_type

        // IntProperty
        write_property_tag(
            &mut buf,
            &PropertyTag {
                name: ascii("Level"),
                type_name: ascii("IntProperty"),
                size: 4,
                index: 0,
                extra: TagExtra::None,
                guid: None,
            },
            true,
        );
        write_i32_le(&mut buf, 42);

        // BoolProperty (value lives in the tag, size == 0)
        write_property_tag(
            &mut buf,
            &PropertyTag {
                name: ascii("bIsActive"),
                type_name: ascii("BoolProperty"),
                size: 0,
                index: 0,
                extra: TagExtra::Bool(true),
                guid: None,
            },
            true,
        );

        // StructProperty carrying an opaque nested payload we never decode here.
        let struct_value = b"\x01\x02\x03\x04\x05\x06\x07\x08".to_vec();
        write_property_tag(
            &mut buf,
            &PropertyTag {
                name: ascii("Location"),
                type_name: ascii("StructProperty"),
                size: struct_value.len() as u32,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii("Vector"),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );
        buf.extend_from_slice(&struct_value);

        // None terminator
        write_fstring(&mut buf, &none_terminator());

        // Trailing bytes after the property list (uesave calls this `extra`).
        buf.extend_from_slice(b"\xAA\xBB\xCC");

        buf
    }

    #[test]
    fn parses_and_indexes_without_decoding_values() {
        let bytes = synthetic_gvas();
        let file = GvasFile::parse(&bytes).expect("parse");

        assert_eq!(file.save_game_type, "PalworldSaveGame");
        assert_eq!(file.properties.len(), 3);
        assert_eq!(file.properties[0].name, "Level");
        assert_eq!(file.properties[0].type_name, "IntProperty");
        assert_eq!(file.properties[1].name, "bIsActive");
        assert_eq!(file.properties[2].name, "Location");
        assert_eq!(file.properties[2].type_name, "StructProperty");
    }

    #[test]
    fn write_is_byte_identical_to_source() {
        let bytes = synthetic_gvas();
        let file = GvasFile::parse(&bytes).expect("parse");
        assert_eq!(file.write(), bytes);
    }

    #[test]
    fn spans_exactly_partition_the_buffer() {
        let bytes = synthetic_gvas();
        let file = GvasFile::parse(&bytes).expect("parse");

        let mut cursor = 0usize;
        assert_eq!(file.header_span.start, cursor);
        cursor = file.header_span.end;
        assert_eq!(file.save_game_type_span.start, cursor);
        cursor = file.save_game_type_span.end;
        for p in &file.properties {
            assert_eq!(p.span.start, cursor);
            cursor = p.span.end;
        }
        assert_eq!(file.none_span.start, cursor);
        cursor = file.none_span.end;
        assert_eq!(file.trailing_span.start, cursor);
        assert_eq!(file.trailing_span.end, bytes.len());
    }

    #[test]
    fn engine_54_plus_is_rejected_not_misparsed() {
        let mut buf = Vec::new();
        write_u32_le(&mut buf, header::GVAS_MAGIC);
        write_u32_le(&mut buf, 3);
        write_u32_le(&mut buf, 522);
        write_u32_le(&mut buf, 1007);
        write_u16_le(&mut buf, 5);
        write_u16_le(&mut buf, 4); // engine minor == 4 -> new tag format
        write_u16_le(&mut buf, 0);
        write_u32_le(&mut buf, 0);
        write_fstring(&mut buf, &FString::Empty);
        write_u32_le(&mut buf, 1);
        write_u32_le(&mut buf, 0);

        let err = GvasFile::parse(&buf).unwrap_err();
        assert!(matches!(
            err,
            GvasError::UnsupportedPropertyTagFormat {
                engine_major: 5,
                engine_minor: 4
            }
        ));
    }

    #[test]
    fn truncated_value_is_an_error_not_a_panic() {
        let mut bytes = synthetic_gvas();
        // Chop off the tail so the last struct's declared 8-byte value doesn't fit.
        bytes.truncate(bytes.len() - 15);
        assert!(GvasFile::parse(&bytes).is_err());
    }
}
