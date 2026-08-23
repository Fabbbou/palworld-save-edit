//! Materializes one property's value bytes into a structured `Value`. Read-only for
//! now — nothing here writes `Value` back to bytes; until Phase 4's splice engine
//! exists, an edited value has nowhere safe to go, so `GvasFile::write` keeps using
//! the verbatim span for every property regardless of whether it's been materialized.
//!
//! Wire layouts are ported from uesave-rs's `Property::read_value` / `ValueVec::read`
//! / `ValueVec::read_array` / `StructValue::read` (trumank/uesave-rs, MIT) for the
//! legacy (pre-5.4) tag format. Anything outside the closed set the project plan
//! calls for (Int, UInt16, UInt32, Int64, Float, Str, Name, Enum, Bool, Byte, Array,
//! Map, Struct) — Sets, Text, Object refs, delegates, arrays/maps of struct types we
//! haven't ported the shared-tag layout for — falls back to `Value::Raw`. So does any
//! internal length mismatch: if decoding a value doesn't land exactly on its declared
//! end offset, that's treated as "we got this wrong," not returned as a half-decoded
//! value. Never guess; degrade to opaque instead.

use super::PropertyEntry;
use super::error::GvasError;
use super::primitives::{
    FString, Guid, read_f32_le, read_f64_le, read_fstring, read_guid, read_i32_le, read_i64_le,
    read_u8, read_u16_le, read_u32_le, read_u64_le,
};
use super::property::{PropertyTag, TagExtra, read_property_tag};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i32),
    UInt16(u16),
    UInt32(u32),
    Int64(i64),
    Float(f32),
    Str(FString),
    Name(FString),
    Bool(bool),
    /// Byte-as-number (tag's enum_type was the literal string "None").
    Byte(u8),
    /// Byte-as-enum-label (tag's enum_type named a real enum).
    ByteLabel(FString),
    Enum(FString),
    /// ArrayProperty<ByteProperty> read as a raw blob rather than Vec<Value> — this is
    /// the wire shape of Palworld's "RawData" properties (Phase 3's actual target).
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Struct(StructValue),
    /// Anything outside the closed set above, or a decode that didn't consume exactly
    /// its declared length. Original value bytes, untouched.
    Raw(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum StructValue {
    Guid(Guid),
    /// FDateTime ticks, no calendar math attempted here.
    DateTime(u64),
    Vector {
        x: f64,
        y: f64,
        z: f64,
    },
    Quat {
        x: f64,
        y: f64,
        z: f64,
        w: f64,
    },
    LinearColor {
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    },
    /// A user-defined struct with no special engine encoding: just a nested,
    /// None-terminated property list, indexed exactly like the top-level one.
    Properties(Vec<PropertyEntry>),
}

/// Typed accessors. Every one returns `Option` and never panics: a caller reading a
/// game field it expected to be an `i32` should degrade to "unknown" rather than
/// fail, because Palworld renames and retypes fields between versions and a screen
/// that refuses to render over one missing stat is worse than one showing a blank.
impl Value {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }

    /// Any integer-ish scalar widened to `i64`. Palworld is inconsistent about widths
    /// for conceptually similar fields — a Pal's `Level` is a `ByteProperty` while its
    /// `Exp` is an `Int64Property` — so most callers want this rather than an
    /// exact-variant match.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Byte(v) => Some(i64::from(*v)),
            Value::UInt16(v) => Some(i64::from(*v)),
            Value::UInt32(v) => Some(i64::from(*v)),
            Value::Int(v) => Some(i64::from(*v)),
            Value::Int64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Value::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_byte(&self) -> Option<u8> {
        match self {
            Value::Byte(v) => Some(*v),
            _ => None,
        }
    }

    /// Lossy display text for any of the string-shaped variants. For display and for
    /// matching known ASCII identifiers — not for byte-exact round-tripping, which
    /// goes through `FString` itself.
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Str(s) | Value::Name(s) | Value::Enum(s) | Value::ByteLabel(s) => {
                Some(s.display_lossy())
            }
            _ => None,
        }
    }

    pub fn as_guid(&self) -> Option<Guid> {
        match self {
            Value::Struct(StructValue::Guid(g)) => Some(*g),
            _ => None,
        }
    }

    /// FDateTime ticks (100ns intervals since 0001-01-01).
    pub fn as_ticks(&self) -> Option<u64> {
        match self {
            Value::Struct(StructValue::DateTime(t)) => Some(*t),
            _ => None,
        }
    }

    pub fn as_properties(&self) -> Option<&[PropertyEntry]> {
        match self {
            Value::Struct(StructValue::Properties(p)) => Some(p),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(Value, Value)]> {
        match self {
            Value::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

fn large_world_coordinates(engine_major: u16) -> bool {
    engine_major >= 5
}

/// Struct-typed map keys and values carry no struct-type name on the wire, so the type
/// is looked up by path (see `gvas::hints`). The fallback when the table has no entry
/// is the same default uesave-rs itself uses: `Guid` for keys, a generic property list
/// for values.
///
/// Factored out because [`map_layout`] has to make the identical choice. If the two
/// ever disagreed, the spans it reports would not line up with the values
/// `try_materialize_value` decodes, and an entry copied between saves would be sliced
/// at the wrong boundary.
fn map_struct_defaults(path: &str) -> (&'static str, &'static str) {
    let key_default = match super::hints::lookup(&format!("{path}.Key")) {
        Some(super::hints::StructHint::Guid) | None => "Guid",
        Some(super::hints::StructHint::Generic) => "Struct",
    };
    let value_default = match super::hints::lookup(&format!("{path}.Value")) {
        Some(super::hints::StructHint::Guid) => "Guid",
        Some(super::hints::StructHint::Generic) | None => "Struct",
    };
    (key_default, value_default)
}

/// Where a `MapProperty`'s parts sit in the buffer, for edits that add or remove
/// entries rather than rewriting a value in place.
///
/// `Value::Map` throws offsets away — it is a decoded view, not a layout — so anything
/// that needs to splice at an entry boundary has to recover them. See
/// [`crate::edit::insert_map_entry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapLayout {
    /// Number of "keys to remove" recorded before the entries. Zero in every Palworld
    /// map seen so far; the edit functions refuse a non-zero one rather than guess.
    pub removed_count: u32,
    /// Offset of the u32 entry count, which an insert or remove has to patch.
    pub entry_count_offset: usize,
    /// `key_start..value_end` for each entry, in wire order. Copying one of these byte
    /// ranges is exactly what moving an entry between saves means.
    pub entries: Vec<Range<usize>>,
}

/// Where an `ArrayProperty`-of-structs' parts sit in the buffer.
///
/// Structurally the array is `[u32 count][inner tag][body]×count`. The inner tag is the
/// complication that maps don't have: it carries a `size` field of its own, so changing
/// the element count means deciding what that field is supposed to say. See
/// `array_inner_tag_size_covers_all_element_bodies` for what the game actually writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayLayout {
    /// Offset of the u32 element count.
    pub count_offset: usize,
    /// Offset of the `size` field inside the array's nested element tag.
    pub inner_size_offset: usize,
    /// What that field currently says.
    pub inner_size: u32,
    /// Byte span of each element body, in wire order.
    pub elements: Vec<Range<usize>>,
}

/// Walks an `ArrayProperty` of structs recording byte boundaries.
///
/// Returns `UnknownPropertyType` for arrays of anything else — `TArray<uint8>`
/// (`RawData`) has no inner tag and no element boundaries worth naming.
pub fn array_layout(
    source: &[u8],
    entry: &PropertyEntry,
    engine_major: u16,
    has_property_guid: bool,
) -> Result<ArrayLayout, GvasError> {
    let mut pos = entry.span.start;
    let tag = read_property_tag(source, &mut pos, has_property_guid)?
        .expect("indexed property span always starts at a real tag, never the None terminator");
    let TagExtra::Array { inner_type } = &tag.extra else {
        return Err(GvasError::UnknownPropertyType {
            name: tag.type_name.display_lossy(),
            at: entry.span.start,
        });
    };
    if inner_type.ascii_str() != Some("StructProperty") {
        return Err(GvasError::UnknownPropertyType {
            name: inner_type.display_lossy(),
            at: entry.span.start,
        });
    }

    let count_offset = pos;
    let count = read_u32_le(source, &mut pos)?;

    let inner_tag_start = pos;
    let inner_tag = read_property_tag(source, &mut pos, has_property_guid)?.ok_or_else(|| {
        GvasError::UnknownPropertyType {
            name: "None".to_string(),
            at: inner_tag_start,
        }
    })?;
    let TagExtra::Struct { struct_type, .. } = &inner_tag.extra else {
        return Err(GvasError::UnknownPropertyType {
            name: "expected nested StructProperty tag in struct array".to_string(),
            at: inner_tag_start,
        });
    };
    let struct_type = struct_type.ascii_str().unwrap_or("").to_string();
    let inner_size_offset = super::property::size_field_offset(source, inner_tag_start)?;

    let mut elements = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let start = pos;
        read_struct_body(
            source,
            &mut pos,
            &struct_type,
            engine_major,
            has_property_guid,
        )?;
        elements.push(start..pos);
    }

    if pos != entry.span.end {
        return Err(GvasError::TrailingBytes {
            at: pos,
            expected: entry.span.end,
        });
    }

    Ok(ArrayLayout {
        count_offset,
        inner_size_offset,
        inner_size: inner_tag.size,
        elements,
    })
}

/// Walks a `MapProperty`'s value region recording byte boundaries.
///
/// Deliberately re-walks rather than threading offsets through `Value`: the decoded
/// tree is the common path and stays free of layout concerns, while this runs only when
/// something is about to be spliced. Both share [`map_struct_defaults`] and
/// `read_value_by_type`, so they cannot disagree about where a value ends.
pub fn map_layout(
    source: &[u8],
    entry: &PropertyEntry,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<MapLayout, GvasError> {
    let mut pos = entry.span.start;
    let tag = read_property_tag(source, &mut pos, has_property_guid)?
        .expect("indexed property span always starts at a real tag, never the None terminator");
    let TagExtra::Map {
        key_type,
        value_type,
    } = &tag.extra
    else {
        return Err(GvasError::UnknownPropertyType {
            name: tag.type_name.display_lossy(),
            at: entry.span.start,
        });
    };

    let (key_default, value_default) = map_struct_defaults(path);

    let removed_count = read_u32_le(source, &mut pos)?;
    for _ in 0..removed_count {
        read_value_by_type(
            source,
            &mut pos,
            key_type,
            key_default,
            engine_major,
            has_property_guid,
        )?;
    }

    let entry_count_offset = pos;
    let entry_count = read_u32_le(source, &mut pos)?;

    let mut entries = Vec::with_capacity(entry_count as usize);
    for _ in 0..entry_count {
        let start = pos;
        read_value_by_type(
            source,
            &mut pos,
            key_type,
            key_default,
            engine_major,
            has_property_guid,
        )?;
        read_value_by_type(
            source,
            &mut pos,
            value_type,
            value_default,
            engine_major,
            has_property_guid,
        )?;
        entries.push(start..pos);
    }

    // The walk must land exactly on the property's declared end. Anything else means
    // the layout was misread, and splicing against it would cut an entry in half.
    if pos != entry.span.end {
        return Err(GvasError::TrailingBytes {
            at: pos,
            expected: entry.span.end,
        });
    }

    Ok(MapLayout {
        removed_count,
        entry_count_offset,
        entries,
    })
}

/// Re-parses `entry`'s tag from `source` and decodes its value. `source` must be the
/// same buffer the entry's span was computed against. `path` is this property's own
/// dotted path from the save root (e.g. `"worldSaveData.GroupSaveDataMap"`) — used
/// only to look up `MapProperty` key/value struct-type hints (see `gvas::hints`); it
/// doesn't affect anything else about how the bytes are read.
pub fn materialize_property(
    source: &[u8],
    entry: &PropertyEntry,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<Value, GvasError> {
    let mut pos = entry.span.start;
    let tag = read_property_tag(source, &mut pos, has_property_guid)?
        .expect("indexed property span always starts at a real tag, never the None terminator");
    materialize_value(
        source,
        &mut pos,
        &tag,
        entry.span.end,
        engine_major,
        has_property_guid,
        path,
    )
}

fn materialize_value(
    buf: &[u8],
    pos: &mut usize,
    tag: &PropertyTag,
    value_end: usize,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<Value, GvasError> {
    let start = *pos;
    let attempt = try_materialize_value(buf, pos, tag, engine_major, has_property_guid, path);
    match attempt {
        Ok(value) if *pos == value_end => Ok(value),
        // Either a decode error, or it decoded something but not exactly the declared
        // length — both mean our understanding of this value's layout is wrong here.
        _ => Ok(Value::Raw(buf[start..value_end].to_vec())),
    }
}

fn try_materialize_value(
    buf: &[u8],
    pos: &mut usize,
    tag: &PropertyTag,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<Value, GvasError> {
    match &tag.extra {
        TagExtra::Bool(b) => Ok(Value::Bool(*b)),
        TagExtra::Byte { enum_type } => {
            if enum_type.ascii_str() == Some("None") {
                Ok(Value::Byte(read_u8(buf, pos)?))
            } else {
                Ok(Value::ByteLabel(read_fstring(buf, pos)?))
            }
        }
        TagExtra::Enum { .. } => Ok(Value::Enum(read_fstring(buf, pos)?)),
        TagExtra::Struct { struct_type, .. } => read_struct_body(
            buf,
            pos,
            struct_type.ascii_str().unwrap_or(""),
            engine_major,
            has_property_guid,
        ),
        TagExtra::Array { inner_type } => {
            let count = read_u32_le(buf, pos)?;
            read_array_body(buf, pos, inner_type, count, engine_major, has_property_guid)
        }
        TagExtra::Map {
            key_type,
            value_type,
        } => {
            let (key_default, value_default) = map_struct_defaults(path);
            let removed_count = read_u32_le(buf, pos)?;
            for _ in 0..removed_count {
                read_value_by_type(
                    buf,
                    pos,
                    key_type,
                    key_default,
                    engine_major,
                    has_property_guid,
                )?;
            }
            let entry_count = read_u32_le(buf, pos)?;
            let mut entries = Vec::with_capacity(entry_count as usize);
            for _ in 0..entry_count {
                let k = read_value_by_type(
                    buf,
                    pos,
                    key_type,
                    key_default,
                    engine_major,
                    has_property_guid,
                )?;
                let v = read_value_by_type(
                    buf,
                    pos,
                    value_type,
                    value_default,
                    engine_major,
                    has_property_guid,
                )?;
                entries.push((k, v));
            }
            Ok(Value::Map(entries))
        }
        // Sets aren't in the plan's closed type set; fall back to opaque.
        TagExtra::Set { .. } => Err(GvasError::UnknownPropertyType {
            name: "SetProperty".to_string(),
            at: *pos,
        }),
        TagExtra::None => match tag.type_name.ascii_str() {
            Some("IntProperty") => Ok(Value::Int(read_i32_le(buf, pos)?)),
            Some("UInt16Property") => Ok(Value::UInt16(read_u16_le(buf, pos)?)),
            Some("UInt32Property") => Ok(Value::UInt32(read_u32_le(buf, pos)?)),
            Some("Int64Property") => Ok(Value::Int64(read_i64_le(buf, pos)?)),
            Some("FloatProperty") => Ok(Value::Float(read_f32_le(buf, pos)?)),
            Some("StrProperty") => Ok(Value::Str(read_fstring(buf, pos)?)),
            Some("NameProperty") => Ok(Value::Name(read_fstring(buf, pos)?)),
            other => Err(GvasError::UnknownPropertyType {
                name: other.unwrap_or("<non-ascii>").to_string(),
                at: *pos,
            }),
        },
    }
}

/// Decodes a value given only its property *type name* (as used for Map keys/values
/// and array elements) rather than a full tag — matches uesave-rs's
/// `Property::read_value` called with `PropertyTagDataFull::from_type(..)`, which is
/// what a Map/Array falls back to for its elements when no schema hint names the real
/// struct type. `struct_default` is that fallback struct type name ("Guid" for map
/// keys, "Struct" — generic property list — for map values and array elements),
/// mirroring uesave-rs's own unhinted defaults; we have no Palworld type-hint table
/// yet (that's Phase 3's PALWORLD_TYPE_HINTS-equivalent), so this isn't a new guess.
fn read_value_by_type(
    buf: &[u8],
    pos: &mut usize,
    type_name: &FString,
    struct_default: &str,
    engine_major: u16,
    has_property_guid: bool,
) -> Result<Value, GvasError> {
    match type_name.ascii_str() {
        Some("StructProperty") => {
            read_struct_body(buf, pos, struct_default, engine_major, has_property_guid)
        }
        Some("BoolProperty") => Ok(Value::Bool(read_u8(buf, pos)? > 0)),
        Some("ByteProperty") => Ok(Value::Byte(read_u8(buf, pos)?)),
        Some("EnumProperty") => Ok(Value::Enum(read_fstring(buf, pos)?)),
        Some("IntProperty") => Ok(Value::Int(read_i32_le(buf, pos)?)),
        Some("UInt16Property") => Ok(Value::UInt16(read_u16_le(buf, pos)?)),
        Some("UInt32Property") => Ok(Value::UInt32(read_u32_le(buf, pos)?)),
        Some("Int64Property") => Ok(Value::Int64(read_i64_le(buf, pos)?)),
        Some("FloatProperty") => Ok(Value::Float(read_f32_le(buf, pos)?)),
        Some("StrProperty") => Ok(Value::Str(read_fstring(buf, pos)?)),
        Some("NameProperty") => Ok(Value::Name(read_fstring(buf, pos)?)),
        other => Err(GvasError::UnknownPropertyType {
            name: other.unwrap_or("<non-ascii>").to_string(),
            at: *pos,
        }),
    }
}

fn read_struct_body(
    buf: &[u8],
    pos: &mut usize,
    struct_type: &str,
    engine_major: u16,
    has_property_guid: bool,
) -> Result<Value, GvasError> {
    let lwc = large_world_coordinates(engine_major);
    match struct_type {
        "Guid" => Ok(Value::Struct(StructValue::Guid(read_guid(buf, pos)?))),
        "DateTime" => Ok(Value::Struct(StructValue::DateTime(read_u64_le(buf, pos)?))),
        "Vector" => {
            let (x, y, z) = if lwc {
                (
                    read_f64_le(buf, pos)?,
                    read_f64_le(buf, pos)?,
                    read_f64_le(buf, pos)?,
                )
            } else {
                (
                    read_f32_le(buf, pos)? as f64,
                    read_f32_le(buf, pos)? as f64,
                    read_f32_le(buf, pos)? as f64,
                )
            };
            Ok(Value::Struct(StructValue::Vector { x, y, z }))
        }
        "Quat" => {
            let (x, y, z, w) = if lwc {
                (
                    read_f64_le(buf, pos)?,
                    read_f64_le(buf, pos)?,
                    read_f64_le(buf, pos)?,
                    read_f64_le(buf, pos)?,
                )
            } else {
                (
                    read_f32_le(buf, pos)? as f64,
                    read_f32_le(buf, pos)? as f64,
                    read_f32_le(buf, pos)? as f64,
                    read_f32_le(buf, pos)? as f64,
                )
            };
            Ok(Value::Struct(StructValue::Quat { x, y, z, w }))
        }
        "LinearColor" => Ok(Value::Struct(StructValue::LinearColor {
            r: read_f32_le(buf, pos)?,
            g: read_f32_le(buf, pos)?,
            b: read_f32_le(buf, pos)?,
            a: read_f32_le(buf, pos)?,
        })),
        // Everything else: a user-defined USTRUCT, serialized as a plain
        // None-terminated property list. This is also uesave-rs's own fallback for
        // any struct name it doesn't special-case, engine-native or not.
        _ => {
            let nested = read_property_list(buf, pos, has_property_guid)?;
            Ok(Value::Struct(StructValue::Properties(nested)))
        }
    }
}

/// Walks a None-terminated property list starting at `*pos`, indexing each entry by
/// span exactly like `GvasFile::parse` does at the top level — just at whatever
/// absolute offset this nested struct happens to live at. No copying: spans are
/// offsets into the same buffer the caller already owns.
///
/// `pub(crate)`, not `pub(super)`: `rawdata` decoders (e.g. `PalCharacterData`) embed
/// a GVAS property list inside an otherwise-opaque RawData blob and need this same
/// walk, on their own byte slice rather than the save's.
pub fn read_property_list(
    buf: &[u8],
    pos: &mut usize,
    has_property_guid: bool,
) -> Result<Vec<PropertyEntry>, GvasError> {
    let mut properties = Vec::new();
    loop {
        let entry_start = *pos;
        match read_property_tag(buf, pos, has_property_guid)? {
            None => break,
            Some(tag) => {
                let value_len = tag.size as usize;
                if buf.len() < *pos + value_len {
                    return Err(GvasError::UnexpectedEof {
                        need: value_len,
                        at: *pos,
                        have: buf.len().saturating_sub(*pos),
                    });
                }
                *pos += value_len;
                properties.push(PropertyEntry {
                    name: tag.name.display_lossy(),
                    type_name: tag.type_name.display_lossy(),
                    span: entry_start..*pos,
                });
            }
        }
    }
    Ok(properties)
}

/// Array value body, called with `*pos` already past the 4-byte element count.
fn read_array_body(
    buf: &[u8],
    pos: &mut usize,
    inner_type: &FString,
    count: u32,
    engine_major: u16,
    has_property_guid: bool,
) -> Result<Value, GvasError> {
    match inner_type.ascii_str() {
        // The common case: TArray<uint8> ("RawData"). Read as a blob, not Vec<Value>
        // — this is the one place a Palworld save keeps genuinely large arrays at the
        // GVAS level, so avoid the per-element enum wrapper overhead.
        Some("ByteProperty") => {
            let mut bytes = Vec::with_capacity(count as usize);
            for _ in 0..count {
                bytes.push(read_u8(buf, pos)?);
            }
            Ok(Value::Bytes(bytes))
        }
        Some("StructProperty") => {
            // Legacy format + array_inner_tag (engine >= 4.12, always true here): the
            // array carries one nested property tag whose Struct extra gives the real
            // element struct type, then that many struct bodies with no further tags.
            // Ported from uesave-rs's `ValueVec::read_array`'s Struct branch.
            //
            // The tag is present even when count == 0 — 35 of the 1488 real
            // `ItemContainerSaveData.Slots` arrays in fixtures/Level.sav are exactly
            // that: a zero count followed by a full, otherwise-unused inner tag and
            // nothing else. Returning early on count == 0 leaves those bytes
            // unconsumed and fails the enclosing length check.
            let inner_tag = read_property_tag(buf, pos, has_property_guid)?.ok_or_else(|| {
                GvasError::UnknownPropertyType {
                    name: "None".to_string(),
                    at: *pos,
                }
            })?;
            let struct_type = match &inner_tag.extra {
                TagExtra::Struct { struct_type, .. } => {
                    struct_type.ascii_str().unwrap_or("").to_string()
                }
                _ => {
                    return Err(GvasError::UnknownPropertyType {
                        name: "expected nested StructProperty tag in struct array".to_string(),
                        at: *pos,
                    });
                }
            };
            let mut elements = Vec::with_capacity(count as usize);
            for _ in 0..count {
                elements.push(read_struct_body(
                    buf,
                    pos,
                    &struct_type,
                    engine_major,
                    has_property_guid,
                )?);
            }
            Ok(Value::Array(elements))
        }
        Some(_) => {
            // StructProperty is handled above, so this never hits the Struct arm —
            // the default name passed here is inert.
            let mut elements = Vec::with_capacity(count as usize);
            for _ in 0..count {
                elements.push(read_value_by_type(
                    buf,
                    pos,
                    inner_type,
                    "Struct",
                    engine_major,
                    has_property_guid,
                )?);
            }
            Ok(Value::Array(elements))
        }
        None => Err(GvasError::UnknownPropertyType {
            name: "<non-ascii array inner type>".to_string(),
            at: *pos,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gvas::primitives::{write_fstring, write_guid, write_i32_le, write_u32_le};
    use crate::gvas::property::{none_terminator, write_property_tag};

    fn ascii(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    /// Builds a standalone property (tag + value bytes) and materializes it, as if it
    /// were one entry in a GvasFile — the entry point every real fixture goes through.
    fn materialize_one(tag: PropertyTag, value_bytes: &[u8]) -> Value {
        let mut buf = Vec::new();
        write_property_tag(&mut buf, &tag, true);
        let tag_end = buf.len();
        buf.extend_from_slice(value_bytes);

        let entry = PropertyEntry {
            name: tag.name.display_lossy(),
            type_name: tag.type_name.display_lossy(),
            span: 0..tag_end + value_bytes.len(),
        };
        materialize_property(&buf, &entry, 5, true, &entry.name).unwrap()
    }

    #[test]
    fn materializes_int() {
        let tag = PropertyTag {
            name: ascii("Level"),
            type_name: ascii("IntProperty"),
            size: 4,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        };
        let mut value_bytes = Vec::new();
        write_i32_le(&mut value_bytes, 42);
        assert_eq!(materialize_one(tag, &value_bytes), Value::Int(42));
    }

    #[test]
    fn materializes_bool_from_tag_not_value_bytes() {
        let tag = PropertyTag {
            name: ascii("bActive"),
            type_name: ascii("BoolProperty"),
            size: 0,
            index: 0,
            extra: TagExtra::Bool(true),
            guid: None,
        };
        assert_eq!(materialize_one(tag, &[]), Value::Bool(true));
    }

    #[test]
    fn materializes_byte_as_raw_number_when_enum_type_is_none() {
        let tag = PropertyTag {
            name: ascii("Flags"),
            type_name: ascii("ByteProperty"),
            size: 1,
            index: 0,
            extra: TagExtra::Byte {
                enum_type: ascii("None"),
            },
            guid: None,
        };
        assert_eq!(materialize_one(tag, &[7]), Value::Byte(7));
    }

    #[test]
    fn materializes_generic_struct_as_nested_property_list() {
        let mut inner = Vec::new();
        write_property_tag(
            &mut inner,
            &PropertyTag {
                name: ascii("InGameDay"),
                type_name: ascii("IntProperty"),
                size: 4,
                index: 0,
                extra: TagExtra::None,
                guid: None,
            },
            true,
        );
        write_i32_le(&mut inner, 12);
        write_fstring(&mut inner, &none_terminator());

        let tag = PropertyTag {
            name: ascii("SaveData"),
            type_name: ascii("StructProperty"),
            size: inner.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("SaveDataStruct"),
                guid: [0u8; 16],
            },
            guid: None,
        };

        let value = materialize_one(tag, &inner);
        let Value::Struct(StructValue::Properties(props)) = value else {
            panic!("expected nested properties")
        };
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].name, "InGameDay");
        assert_eq!(props[0].type_name, "IntProperty");
    }

    #[test]
    fn materializes_guid_struct() {
        let mut value_bytes = Vec::new();
        let guid: Guid = [9u8; 16];
        write_guid(&mut value_bytes, &guid);

        let tag = PropertyTag {
            name: ascii("Id"),
            type_name: ascii("StructProperty"),
            size: 16,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("Guid"),
                guid: [0u8; 16],
            },
            guid: None,
        };
        assert_eq!(
            materialize_one(tag, &value_bytes),
            Value::Struct(StructValue::Guid(guid))
        );
    }

    #[test]
    fn materializes_byte_array_as_raw_bytes() {
        let mut value_bytes = Vec::new();
        write_u32_le(&mut value_bytes, 3); // count
        value_bytes.extend_from_slice(&[1, 2, 3]);

        let tag = PropertyTag {
            name: ascii("RawData"),
            type_name: ascii("ArrayProperty"),
            size: value_bytes.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("ByteProperty"),
            },
            guid: None,
        };
        assert_eq!(
            materialize_one(tag, &value_bytes),
            Value::Bytes(vec![1, 2, 3])
        );
    }

    /// Regression: a zero-length array of structs still carries its inner element
    /// tag on the wire, and that tag's bytes must be consumed. 35 of the 1488 real
    /// `ItemContainerSaveData.Slots` arrays in fixtures/Level.sav look exactly like
    /// this; returning early on count == 0 left the tag unread and the whole
    /// property fell back to `Value::Raw`.
    #[test]
    fn materializes_empty_struct_array_including_its_inner_tag() {
        let mut value_bytes = Vec::new();
        write_u32_le(&mut value_bytes, 0); // count == 0
        write_property_tag(
            &mut value_bytes,
            &PropertyTag {
                name: ascii("Slots"),
                type_name: ascii("StructProperty"),
                size: 0,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii("PalItemSlotSaveData"),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );

        let tag = PropertyTag {
            name: ascii("Slots"),
            type_name: ascii("ArrayProperty"),
            size: value_bytes.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("StructProperty"),
            },
            guid: None,
        };
        assert_eq!(materialize_one(tag, &value_bytes), Value::Array(vec![]));
    }

    #[test]
    fn materializes_map_with_guid_keys() {
        let mut value_bytes = Vec::new();
        write_u32_le(&mut value_bytes, 0); // removed_count
        write_u32_le(&mut value_bytes, 1); // entry_count
        let key: Guid = [3u8; 16];
        write_guid(&mut value_bytes, &key);
        write_i32_le(&mut value_bytes, 99); // Int value

        let tag = PropertyTag {
            name: ascii("Scores"),
            type_name: ascii("MapProperty"),
            size: value_bytes.len() as u32,
            index: 0,
            extra: TagExtra::Map {
                key_type: ascii("StructProperty"),
                value_type: ascii("IntProperty"),
            },
            guid: None,
        };

        let value = materialize_one(tag, &value_bytes);
        let Value::Map(entries) = value else {
            panic!("expected a map")
        };
        assert_eq!(
            entries,
            vec![(Value::Struct(StructValue::Guid(key)), Value::Int(99))]
        );
    }

    #[test]
    fn falls_back_to_raw_on_unknown_type_instead_of_erroring() {
        let tag = PropertyTag {
            name: ascii("Weird"),
            type_name: ascii("ObjectProperty"), // not in our closed set
            size: 4,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        };
        let value_bytes = [0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(
            materialize_one(tag, &value_bytes),
            Value::Raw(value_bytes.to_vec())
        );
    }
}
