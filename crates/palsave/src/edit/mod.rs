//! Splice engine: apply edits without re-serializing the save.
//!
//! Every edit is expressed as a set of byte-range replacements against the original
//! decompressed GVAS buffer. `SpliceSet::apply` sorts them, rejects overlaps, and
//! assembles the result as a chunk list plus one final copy — untouched regions are
//! `memcpy`'d straight from the source and never re-encoded. Editing one guild
//! rewrites that guild's `RawData` blob and patches a handful of 4-byte `size`
//! fields; the multi-megabyte `MapObjectSaveData` sibling is copied verbatim.
//!
//! ## Why size fixups are the whole problem
//!
//! A GVAS property tag carries a u32 `size`: the byte length of the value that
//! follows it. Change a value's length and every *enclosing* property's `size` is
//! now wrong, all the way up to the top-level property. Nothing else in the format
//! needs adjusting for an in-place value edit — struct bodies are None-terminated
//! (no length prefix), and map entries have no per-entry length either — so the
//! complete fixup is: patch the `size` field of the edited property and of each of
//! its ancestors by the same delta. `replace_property_value` does exactly that,
//! given the ancestor chain.
//!
//! Adding or removing a **map entry** additionally changes the map's u32 entry count;
//! `insert_map_entry` and `remove_map_entry` handle that. Array elements are a
//! different problem — an `ArrayProperty` of structs writes one inner tag carrying its
//! own `size`, so an element-count change has a fixup this module has not verified —
//! and are still unsupported.

pub mod error;

pub use error::EditError;

use crate::gvas::PropertyEntry;
use crate::gvas::primitives::{FString, write_fstring};
use crate::gvas::property::{PropertyTag, TagExtra};
use crate::gvas::property::{size_field_offset, value_offset};
use crate::gvas::value::map_layout;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct SpliceSet {
    splices: Vec<Splice>,
}

impl SpliceSet {
    pub fn new() -> Self {
        SpliceSet {
            splices: Vec::new(),
        }
    }

    pub fn replace(&mut self, range: Range<usize>, bytes: Vec<u8>) {
        self.splices.push(Splice { range, bytes });
    }

    pub fn is_empty(&self) -> bool {
        self.splices.is_empty()
    }

    pub fn merge(&mut self, other: SpliceSet) {
        self.splices.extend(other.splices);
    }

    /// Assembles the edited buffer. Splices are applied in source order; overlapping
    /// ranges are rejected rather than silently resolved, since which one "wins"
    /// would be arbitrary and the result would be a corrupt save.
    pub fn apply(mut self, source: &[u8]) -> Result<Vec<u8>, EditError> {
        self.splices.sort_by_key(|s| s.range.start);

        for pair in self.splices.windows(2) {
            if pair[0].range.end > pair[1].range.start {
                return Err(EditError::OverlappingSplices {
                    first: pair[0].range.clone(),
                    second: pair[1].range.clone(),
                });
            }
        }
        if let Some(last) = self.splices.last()
            && last.range.end > source.len()
        {
            return Err(EditError::SpliceOutOfBounds {
                range: last.range.clone(),
                source_len: source.len(),
            });
        }

        let mut out = Vec::with_capacity(source.len());
        let mut cursor = 0usize;
        for splice in &self.splices {
            out.extend_from_slice(&source[cursor..splice.range.start]);
            out.extend_from_slice(&splice.bytes);
            cursor = splice.range.end;
        }
        out.extend_from_slice(&source[cursor..]);
        Ok(out)
    }
}

/// Encodes a `TArray<uint8>` property value: a u32 element count followed by the
/// bytes themselves. This is the wire shape of every Palworld `RawData` property, so
/// it's what a re-encoded `rawdata` blob has to be wrapped in before splicing.
pub fn byte_array_value(blob: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + blob.len());
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(blob);
    out
}

/// Replaces one property's entire value region and patches the `size` field of that
/// property's own tag plus every ancestor's.
///
/// `chain` runs outermost-first and must end with the property being edited — e.g.
/// `[worldSaveData, GroupSaveDataMap, RawData]`. Each entry must physically contain
/// the next; that's checked, not assumed, because a caller assembling the chain from
/// separate `materialize` calls could easily pass siblings by mistake and the
/// resulting save would be quietly corrupt.
pub fn replace_property_value(
    source: &[u8],
    chain: &[&PropertyEntry],
    new_value: Vec<u8>,
    has_property_guid: bool,
) -> Result<SpliceSet, EditError> {
    let Some(leaf) = chain.last() else {
        return Err(EditError::EmptyChain);
    };
    check_nesting(chain)?;

    let value_start = value_offset(source, leaf.span.start, has_property_guid)?;
    let old_value_len = leaf.span.end - value_start;
    let delta = new_value.len() as i64 - old_value_len as i64;

    let mut set = SpliceSet::new();
    set.replace(value_start..leaf.span.end, new_value);
    set.merge(size_fixups(source, chain, delta)?);

    Ok(set)
}

/// Checks that each entry in `chain` physically contains the next.
///
/// Assembling a chain from separate `materialize` calls makes it easy to pass siblings
/// by mistake, and the resulting save would be quietly corrupt — so this is checked,
/// never assumed.
fn check_nesting(chain: &[&PropertyEntry]) -> Result<(), EditError> {
    for pair in chain.windows(2) {
        let (outer, inner) = (&pair[0].span, &pair[1].span);
        if !(outer.start <= inner.start && inner.end <= outer.end) {
            return Err(EditError::NotNested {
                outer: outer.clone(),
                inner: inner.clone(),
            });
        }
    }
    Ok(())
}

/// Patches the `size` field of every property in `chain` by `delta`.
///
/// This is the whole of what an edit owes the format when a region's length changes —
/// see the module docs. Shared by the value-replacing and entry-inserting paths so
/// they cannot drift on which sizes get fixed.
fn size_fixups(
    source: &[u8],
    chain: &[&PropertyEntry],
    delta: i64,
) -> Result<SpliceSet, EditError> {
    let mut set = SpliceSet::new();
    if delta == 0 {
        return Ok(set);
    }
    for entry in chain {
        let offset = size_field_offset(source, entry.span.start)?;
        let old_size = u32::from_le_bytes(
            source
                .get(offset..offset + 4)
                .ok_or(EditError::SpliceOutOfBounds {
                    range: offset..offset + 4,
                    source_len: source.len(),
                })?
                .try_into()
                .unwrap(),
        );
        let new_size = i64::from(old_size) + delta;
        let new_size = u32::try_from(new_size).map_err(|_| EditError::SizeOutOfRange {
            offset,
            old_size,
            delta,
        })?;
        set.replace(offset..offset + 4, new_size.to_le_bytes().to_vec());
    }
    Ok(set)
}

/// The raw bytes of one map entry — a key immediately followed by its value, exactly
/// as they sit on the wire.
///
/// This is the unit that moves between saves. It is deliberately opaque: copying a
/// player's `CharacterSaveParameterMap` entry into another world does not require
/// understanding a single field inside it, only that both worlds agree on the map's
/// key and value *types*, which [`insert_map_entry`]'s caller is responsible for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapEntryBytes(pub Vec<u8>);

/// Extracts one entry's bytes so it can be inserted into another save.
pub fn map_entry_bytes(
    source: &[u8],
    map: &PropertyEntry,
    index: usize,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<MapEntryBytes, EditError> {
    let layout = map_layout(source, map, engine_major, has_property_guid, path)?;
    let span = layout
        .entries
        .get(index)
        .ok_or(EditError::MapEntryOutOfRange {
            index,
            count: layout.entries.len(),
        })?;
    Ok(MapEntryBytes(source[span.clone()].to_vec()))
}

/// Refuses a map that records pending key removals.
///
/// Checked by reading the count directly, *before* the layout walk: a map with removals
/// would also make the walk land somewhere unexpected, and "this shape isn't supported"
/// is a far more useful answer than "the layout didn't add up". No Palworld map seen
/// has any — `map_layouts_have_no_pending_key_removals` asserts that across both
/// worlds — so this is a guard against an unseen shape, not a live case.
fn reject_removed_keys(
    source: &[u8],
    map: &PropertyEntry,
    has_property_guid: bool,
) -> Result<(), EditError> {
    let value_start = value_offset(source, map.span.start, has_property_guid)?;
    let bytes = source
        .get(value_start..value_start + 4)
        .ok_or(EditError::SpliceOutOfBounds {
            range: value_start..value_start + 4,
            source_len: source.len(),
        })?;
    let count = u32::from_le_bytes(bytes.try_into().unwrap());
    if count != 0 {
        return Err(EditError::MapHasRemovedKeys { count });
    }
    Ok(())
}

/// How many entries a map holds, counted from its wire layout rather than from a
/// decoded view. `world::open_map` skips entries whose value isn't a property list, so
/// its `entries.len()` is a lower bound; index arithmetic for a splice needs this one.
pub fn map_layout_entry_count(
    source: &[u8],
    map: &PropertyEntry,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> usize {
    map_layout(source, map, engine_major, has_property_guid, path)
        .map(|l| l.entries.len())
        .unwrap_or(0)
}

/// Appends an entry to a `MapProperty`, patching the entry count and enclosing sizes.
///
/// `chain` runs outermost-first and must end with the map itself — e.g.
/// `[worldSaveData, CharacterSaveParameterMap]`. Unlike
/// [`replace_property_value`] the leaf is not being rewritten; bytes are inserted at
/// its end, which is what makes this the operation the module docs used to list as
/// unsupported.
///
/// Three things change, and the whole correctness argument is that there is no fourth:
///
/// 1. the entry's bytes go in at the end of the map's value region,
/// 2. the u32 entry count goes up by one,
/// 3. every enclosing `size` grows by the entry's length.
///
/// Map entries carry no per-entry length and the map's value region has no terminator,
/// so appending needs nothing else. `verify_reparses` is what actually proves that on
/// each edit — a missed fixup makes the buffer stop being an exact partition.
///
/// **Duplicate keys are not checked.** This layer cannot compare keys without knowing
/// their type, and a map with two identical keys is a corrupt save that reads back
/// fine. Callers that can compare keys must do so; `characters::import_*` does.
pub fn insert_map_entry(
    source: &[u8],
    chain: &[&PropertyEntry],
    entry: &MapEntryBytes,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<SpliceSet, EditError> {
    let Some(map) = chain.last() else {
        return Err(EditError::EmptyChain);
    };
    check_nesting(chain)?;

    reject_removed_keys(source, map, has_property_guid)?;
    let layout = map_layout(source, map, engine_major, has_property_guid, path)?;

    let new_count =
        u32::try_from(layout.entries.len() + 1).map_err(|_| EditError::SizeOutOfRange {
            offset: layout.entry_count_offset,
            old_size: layout.entries.len() as u32,
            delta: 1,
        })?;

    let mut set = SpliceSet::new();
    // Zero-length range at the map's end: pure insertion, nothing overwritten.
    set.replace(map.span.end..map.span.end, entry.0.clone());
    set.replace(
        layout.entry_count_offset..layout.entry_count_offset + 4,
        new_count.to_le_bytes().to_vec(),
    );
    set.merge(size_fixups(source, chain, entry.0.len() as i64)?);

    Ok(set)
}

/// Removes one entry from a `MapProperty`. The inverse of [`insert_map_entry`].
pub fn remove_map_entry(
    source: &[u8],
    chain: &[&PropertyEntry],
    index: usize,
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Result<SpliceSet, EditError> {
    let Some(map) = chain.last() else {
        return Err(EditError::EmptyChain);
    };
    check_nesting(chain)?;

    reject_removed_keys(source, map, has_property_guid)?;
    let layout = map_layout(source, map, engine_major, has_property_guid, path)?;

    let span = layout
        .entries
        .get(index)
        .ok_or(EditError::MapEntryOutOfRange {
            index,
            count: layout.entries.len(),
        })?
        .clone();

    let mut set = SpliceSet::new();
    set.replace(span.clone(), Vec::new());
    set.replace(
        layout.entry_count_offset..layout.entry_count_offset + 4,
        ((layout.entries.len() - 1) as u32).to_le_bytes().to_vec(),
    );
    set.merge(size_fixups(source, chain, -(span.len() as i64))?);

    Ok(set)
}

/// A value to write into an existing scalar property.
///
/// Deliberately not a general `Value` -> bytes function: this exists to swap one
/// scalar for another of the *same declared type*, which is the only edit the splice
/// engine supports without count fixups.
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Byte(u8),
    Int(i32),
    Int64(i64),
    Float(f32),
    /// Encoded as an `FString`; ASCII gets the 1-byte form, anything else UTF-16LE.
    Text(String),
}

impl Scalar {
    fn kind(&self) -> &'static str {
        match self {
            Scalar::Byte(_) => "byte",
            Scalar::Int(_) => "int",
            Scalar::Int64(_) => "int64",
            Scalar::Float(_) => "float",
            Scalar::Text(_) => "string",
        }
    }
}

/// Encodes `value` as the property's **declared** type, taken from the tag on disk.
///
/// The type is never inferred from the caller's `Scalar` — a mismatch is an error.
/// Writing an `i32` into a `ByteProperty` would produce a save that still parses
/// (the size field would be wrong by three bytes and the next property would be read
/// from the wrong offset) and would be found only when the game refused to load it.
pub fn encode_scalar(tag: &PropertyTag, value: &Scalar) -> Result<Vec<u8>, EditError> {
    let property = tag.name.display_lossy();
    let declared = tag.type_name.display_lossy();
    let mismatch = || EditError::TypeMismatch {
        property: property.clone(),
        declared: declared.clone(),
        given: value.kind(),
    };

    let mut out = Vec::new();
    match declared.as_str() {
        // A ByteProperty carries an enum-name in its tag; when that names a real enum
        // the value is an FString label, not a number. Only the numeric form is
        // writable here.
        "ByteProperty" => match (&tag.extra, value) {
            (TagExtra::Byte { enum_type }, Scalar::Byte(v))
                if enum_type.ascii_str() == Some("None") =>
            {
                out.push(*v)
            }
            (TagExtra::Byte { .. }, Scalar::Byte(_)) => {
                return Err(EditError::UnsupportedPropertyType {
                    property,
                    declared: format!("{declared} (enum-labelled)"),
                });
            }
            _ => return Err(mismatch()),
        },
        "IntProperty" => match value {
            Scalar::Int(v) => out.extend_from_slice(&v.to_le_bytes()),
            _ => return Err(mismatch()),
        },
        "Int64Property" => match value {
            Scalar::Int64(v) => out.extend_from_slice(&v.to_le_bytes()),
            _ => return Err(mismatch()),
        },
        "FloatProperty" => match value {
            Scalar::Float(v) => out.extend_from_slice(&v.to_le_bytes()),
            _ => return Err(mismatch()),
        },
        "StrProperty" | "NameProperty" => match value {
            Scalar::Text(v) => write_fstring(&mut out, &encode_text(v)),
            _ => return Err(mismatch()),
        },
        // BoolProperty stores its value in the tag (size == 0), so it needs a
        // different splice entirely. See the enum's doc comment.
        _ => {
            return Err(EditError::UnsupportedPropertyType { property, declared });
        }
    }
    Ok(out)
}

/// ASCII uses the 1-byte-per-char form, anything else UTF-16LE; both carry the null
/// terminator the format counts in its length field. Mirrors `FString`'s own
/// round-trip rules in `gvas::primitives`.
fn encode_text(text: &str) -> FString {
    if text.is_empty() {
        FString::Empty
    } else if text.is_ascii() {
        FString::Ascii {
            content: text.as_bytes().to_vec(),
            trailing: vec![0],
        }
    } else {
        FString::Utf16 {
            content: text.encode_utf16().collect(),
            trailing: vec![0, 0],
        }
    }
}

/// Structural verification for an edited buffer, to be run before handing bytes back
/// to a caller. Re-parses the GVAS and asserts the parse partitions the buffer
/// exactly — which is only true if every `size` field agrees with the actual byte
/// layout, at every level. A missed or miscomputed fixup fails here rather than
/// silently producing a save the game will reject or misread.
///
/// `CLAUDE.md`: "Losing a world to a silent encoder bug is the failure mode that
/// actually matters." This is the check that makes that failure loud.
pub fn verify_reparses(bytes: &[u8]) -> Result<(), EditError> {
    let file = crate::gvas::GvasFile::parse(bytes)?;
    if file.write() != bytes {
        return Err(EditError::VerificationFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gvas::primitives::write_u32_le;
    use crate::gvas::property::{none_terminator, write_property_tag};

    #[test]
    fn apply_splices_in_source_order() {
        let source = b"hello world".to_vec();
        let mut set = SpliceSet::new();
        // Deliberately out of order — apply() sorts.
        set.replace(6..11, b"there".to_vec());
        set.replace(0..5, b"HI".to_vec());
        assert_eq!(set.apply(&source).unwrap(), b"HI there".to_vec());
    }

    #[test]
    fn apply_with_no_splices_is_the_identity() {
        let source = b"unchanged".to_vec();
        assert_eq!(SpliceSet::new().apply(&source).unwrap(), source);
    }

    #[test]
    fn overlapping_splices_are_rejected() {
        let source = b"hello world".to_vec();
        let mut set = SpliceSet::new();
        set.replace(0..6, b"a".to_vec());
        set.replace(3..8, b"b".to_vec());
        assert!(matches!(
            set.apply(&source),
            Err(EditError::OverlappingSplices { .. })
        ));
    }

    #[test]
    fn out_of_bounds_splice_is_rejected() {
        let source = b"short".to_vec();
        let mut set = SpliceSet::new();
        set.replace(0..99, b"x".to_vec());
        assert!(matches!(
            set.apply(&source),
            Err(EditError::SpliceOutOfBounds { .. })
        ));
    }

    #[test]
    fn byte_array_value_prefixes_the_count() {
        assert_eq!(byte_array_value(&[7, 8, 9]), vec![3, 0, 0, 0, 7, 8, 9]);
        assert_eq!(byte_array_value(&[]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn empty_chain_is_rejected() {
        assert!(matches!(
            replace_property_value(b"", &[], vec![], true),
            Err(EditError::EmptyChain)
        ));
    }

    #[test]
    fn non_nested_chain_is_rejected() {
        let outer = PropertyEntry {
            name: "a".into(),
            type_name: "IntProperty".into(),
            span: 0..10,
        };
        let sibling = PropertyEntry {
            name: "b".into(),
            type_name: "IntProperty".into(),
            span: 20..30,
        };
        let err =
            replace_property_value(&[0u8; 64], &[&outer, &sibling], vec![], true).unwrap_err();
        assert!(matches!(err, EditError::NotNested { .. }));
    }

    /// A map whose entries are bare guids, built by hand so these run without fixtures.
    /// `hints` has no entry for this path, so the default applies: Guid keys, generic
    /// struct values — and an empty property list is just a `None` terminator.
    fn guid_map(entries: &[[u8; 16]]) -> (Vec<u8>, PropertyEntry) {
        let mut value = Vec::new();
        write_u32_le(&mut value, 0); // no pending key removals
        write_u32_le(&mut value, entries.len() as u32);
        for key in entries {
            value.extend_from_slice(key);
            write_fstring(&mut value, &none_terminator()); // empty value struct
        }

        let mut buf = Vec::new();
        write_property_tag(
            &mut buf,
            &PropertyTag {
                name: ascii_fstring("M"),
                type_name: ascii_fstring("MapProperty"),
                size: value.len() as u32,
                index: 0,
                extra: TagExtra::Map {
                    key_type: ascii_fstring("StructProperty"),
                    value_type: ascii_fstring("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        let span_start = 0;
        buf.extend_from_slice(&value);
        let entry = PropertyEntry {
            name: "M".into(),
            type_name: "MapProperty".into(),
            span: span_start..buf.len(),
        };
        (buf, entry)
    }

    fn ascii_fstring(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    #[test]
    fn map_entry_bytes_slices_at_entry_boundaries() {
        let (buf, map) = guid_map(&[[1u8; 16], [2u8; 16]]);
        let second = map_entry_bytes(&buf, &map, 1, 5, true, "M").unwrap();
        assert_eq!(&second.0[..16], &[2u8; 16]);

        let err = map_entry_bytes(&buf, &map, 2, 5, true, "M").unwrap_err();
        assert!(matches!(
            err,
            EditError::MapEntryOutOfRange { index: 2, count: 2 }
        ));
    }

    #[test]
    fn insert_then_remove_round_trips_on_a_hand_built_map() {
        let (buf, map) = guid_map(&[[1u8; 16], [2u8; 16]]);
        let chain = [&map];

        let entry = MapEntryBytes({
            let mut v = [3u8; 16].to_vec();
            write_fstring(&mut v, &none_terminator());
            v
        });

        let bigger = insert_map_entry(&buf, &chain, &entry, 5, true, "M")
            .unwrap()
            .apply(&buf)
            .unwrap();
        assert_eq!(bigger.len(), buf.len() + entry.0.len());

        let bigger_map = PropertyEntry {
            span: map.span.start..map.span.end + entry.0.len(),
            ..map.clone()
        };
        let layout = map_layout(&bigger, &bigger_map, 5, true, "M").unwrap();
        assert_eq!(layout.entries.len(), 3);

        let restored = remove_map_entry(&bigger, &[&bigger_map], 2, 5, true, "M")
            .unwrap()
            .apply(&bigger)
            .unwrap();
        assert_eq!(restored, buf);
    }

    /// Removing from the middle has to shift the tail, which is the case an
    /// append-only implementation would get wrong.
    #[test]
    fn removing_a_middle_entry_keeps_the_others_in_order() {
        let (buf, map) = guid_map(&[[1u8; 16], [2u8; 16], [3u8; 16]]);
        // Take the removed entry's length from the layout rather than recomputing it —
        // the test should not carry its own model of how wide an entry is.
        let removed_len = map_layout(&buf, &map, 5, true, "M").unwrap().entries[1].len();

        let smaller = remove_map_entry(&buf, &[&map], 1, 5, true, "M")
            .unwrap()
            .apply(&buf)
            .unwrap();

        let smaller_map = PropertyEntry {
            span: map.span.start..map.span.end - removed_len,
            ..map.clone()
        };
        let layout = map_layout(&smaller, &smaller_map, 5, true, "M").unwrap();
        assert_eq!(layout.entries.len(), 2);
        assert_eq!(&smaller[layout.entries[0].start..][..16], &[1u8; 16]);
        assert_eq!(&smaller[layout.entries[1].start..][..16], &[3u8; 16]);
    }

    #[test]
    fn a_map_with_pending_key_removals_is_refused() {
        let (mut buf, map) = guid_map(&[[1u8; 16]]);
        // Flip removed_count to 1 without adding the key it implies. The point is the
        // refusal, which must happen before anything is written.
        let value_start = value_offset(&buf, map.span.start, true).unwrap();
        buf[value_start..value_start + 4].copy_from_slice(&1u32.to_le_bytes());

        let err =
            insert_map_entry(&buf, &[&map], &MapEntryBytes(vec![0; 4]), 5, true, "M").unwrap_err();
        assert!(matches!(err, EditError::MapHasRemovedKeys { count: 1 }));
    }
}
