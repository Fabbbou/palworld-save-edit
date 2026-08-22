//! Legacy (pre-engine-5.4) property tag header: fstring name, fstring type, u32 size,
//! u32 array index, a type-specific extra block, then an optional property GUID.
//! Ported from uesave-rs's `PropertyTagFull::read`/`write`, the non-`property_tag()`
//! branch. `size` is the exact byte length of the value that follows the tag — true
//! for every property type including collections, which is what makes lazy top-level
//! indexing possible without understanding a property's internal layout.

use super::error::GvasError;
use super::primitives::{
    FString, Guid, read_fstring, read_guid, read_optional_guid, read_u8, read_u32_le,
    write_fstring, write_guid, write_optional_guid, write_u8, write_u32_le,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagExtra {
    None,
    Bool(bool),
    Byte {
        enum_type: FString,
    },
    Enum {
        enum_type: FString,
    },
    Array {
        inner_type: FString,
    },
    Set {
        key_type: FString,
    },
    Map {
        key_type: FString,
        value_type: FString,
    },
    Struct {
        struct_type: FString,
        guid: Guid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyTag {
    pub name: FString,
    pub type_name: FString,
    pub size: u32,
    pub index: u32,
    pub extra: TagExtra,
    pub guid: Option<Guid>,
}

/// Reads one property tag, or `None` if this was the list-terminating "None" marker.
/// `has_property_guid` should come from `Header::has_property_guid()`.
pub fn read_property_tag(
    buf: &[u8],
    pos: &mut usize,
    has_property_guid: bool,
) -> Result<Option<PropertyTag>, GvasError> {
    let name = read_fstring(buf, pos)?;
    if name.ascii_str() == Some("None") {
        return Ok(None);
    }

    let type_name = read_fstring(buf, pos)?;
    let size = read_u32_le(buf, pos)?;
    let index = read_u32_le(buf, pos)?;

    let extra = match type_name.ascii_str() {
        Some("BoolProperty") => TagExtra::Bool(read_u8(buf, pos)? > 0),
        Some("ByteProperty") => TagExtra::Byte {
            enum_type: read_fstring(buf, pos)?,
        },
        Some("EnumProperty") => TagExtra::Enum {
            enum_type: read_fstring(buf, pos)?,
        },
        Some("ArrayProperty") => TagExtra::Array {
            inner_type: read_fstring(buf, pos)?,
        },
        Some("SetProperty") => TagExtra::Set {
            key_type: read_fstring(buf, pos)?,
        },
        Some("MapProperty") => TagExtra::Map {
            key_type: read_fstring(buf, pos)?,
            value_type: read_fstring(buf, pos)?,
        },
        Some("StructProperty") => TagExtra::Struct {
            struct_type: read_fstring(buf, pos)?,
            guid: read_guid(buf, pos)?,
        },
        _ => TagExtra::None,
    };

    let guid = if has_property_guid {
        read_optional_guid(buf, pos)?
    } else {
        None
    };

    Ok(Some(PropertyTag {
        name,
        type_name,
        size,
        index,
        extra,
        guid,
    }))
}

/// Byte offset of a tag's `size` field — the u32 giving the length of the value that
/// follows the tag. `span_start` is a `PropertyEntry`'s span start. The splice engine
/// (`crate::edit`) patches this field in place when an edit changes a value's length,
/// so it needs the offset without re-encoding the whole tag.
pub fn size_field_offset(buf: &[u8], span_start: usize) -> Result<usize, GvasError> {
    let mut pos = span_start;
    read_fstring(buf, &mut pos)?; // name
    read_fstring(buf, &mut pos)?; // type_name
    Ok(pos)
}

/// Byte offset where a property's *value* begins — i.e. one past the end of its tag.
pub fn value_offset(
    buf: &[u8],
    span_start: usize,
    has_property_guid: bool,
) -> Result<usize, GvasError> {
    let mut pos = span_start;
    read_property_tag(buf, &mut pos, has_property_guid)?;
    Ok(pos)
}

pub fn write_property_tag(out: &mut Vec<u8>, tag: &PropertyTag, has_property_guid: bool) {
    write_fstring(out, &tag.name);
    write_fstring(out, &tag.type_name);
    write_u32_le(out, tag.size);
    write_u32_le(out, tag.index);
    match &tag.extra {
        TagExtra::None => {}
        TagExtra::Bool(v) => write_u8(out, *v as u8),
        TagExtra::Byte { enum_type } => write_fstring(out, enum_type),
        TagExtra::Enum { enum_type } => write_fstring(out, enum_type),
        TagExtra::Array { inner_type } => write_fstring(out, inner_type),
        TagExtra::Set { key_type } => write_fstring(out, key_type),
        TagExtra::Map {
            key_type,
            value_type,
        } => {
            write_fstring(out, key_type);
            write_fstring(out, value_type);
        }
        TagExtra::Struct { struct_type, guid } => {
            write_fstring(out, struct_type);
            write_guid(out, guid);
        }
    }
    if has_property_guid {
        write_optional_guid(out, &tag.guid);
    }
}

/// Writes the list-terminating "None" marker (a plain ASCII fstring, no trailing
/// garbage) — used when synthesizing a property list rather than passing one through.
pub fn none_terminator() -> FString {
    FString::Ascii {
        content: b"None".to_vec(),
        trailing: vec![0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(tag: &PropertyTag, has_property_guid: bool) -> PropertyTag {
        let mut buf = Vec::new();
        write_property_tag(&mut buf, tag, has_property_guid);
        let mut pos = 0;
        read_property_tag(&buf, &mut pos, has_property_guid)
            .unwrap()
            .unwrap()
    }

    fn ascii(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    #[test]
    fn none_terminator_is_recognized() {
        let mut buf = Vec::new();
        write_fstring(&mut buf, &none_terminator());
        let mut pos = 0;
        assert!(read_property_tag(&buf, &mut pos, true).unwrap().is_none());
    }

    #[test]
    fn int_property_round_trips() {
        let tag = PropertyTag {
            name: ascii("Level"),
            type_name: ascii("IntProperty"),
            size: 4,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        };
        assert_eq!(round_trip(&tag, true), tag);
    }

    #[test]
    fn bool_property_round_trips() {
        let tag = PropertyTag {
            name: ascii("bIsActive"),
            type_name: ascii("BoolProperty"),
            size: 0,
            index: 0,
            extra: TagExtra::Bool(true),
            guid: None,
        };
        assert_eq!(round_trip(&tag, true), tag);
    }

    #[test]
    fn struct_property_round_trips_with_guid() {
        let tag = PropertyTag {
            name: ascii("Location"),
            type_name: ascii("StructProperty"),
            size: 24,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("Vector"),
                guid: [0u8; 16],
            },
            guid: Some([1u8; 16]),
        };
        assert_eq!(round_trip(&tag, true), tag);
    }

    #[test]
    fn map_property_round_trips() {
        let tag = PropertyTag {
            name: ascii("GroupSaveDataMap"),
            type_name: ascii("MapProperty"),
            size: 100,
            index: 0,
            extra: TagExtra::Map {
                key_type: ascii("StructProperty"),
                value_type: ascii("StructProperty"),
            },
            guid: None,
        };
        assert_eq!(round_trip(&tag, true), tag);
    }

    #[test]
    fn pre_412_header_omits_property_guid_field() {
        let tag = PropertyTag {
            name: ascii("Level"),
            type_name: ascii("IntProperty"),
            size: 4,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        };
        assert_eq!(round_trip(&tag, false), tag);
    }
}
