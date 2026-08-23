//! `.worldSaveData.CharacterSaveParameterMap` value's "RawData" blob: players and
//! Pals. Ported from `oMaN-Rod/uesave-rs` (branch `pluggable-game-support`, MIT),
//! `uesave/src/games/palworld/character.rs`'s `PalCharacterData` — the same
//! actively-maintained fork ADR-002.md credits for the current `GroupSaveDataMap`
//! layout. Matches the project plan's original description of this path almost
//! exactly ("nested property list, then 4 unknown bytes, then group_id guid; the
//! sub-reader must hit EOF exactly") — the one correction is a further 4 trailing
//! bytes after `group_id` that the plan's cheahjs-derived spec didn't have.
//!
//! Unlike `GroupSaveDataMap`'s RawData (pure binary), this blob's first section is
//! itself a nested, None-terminated GVAS property list — the actual stats, IVs,
//! passives, level, etc. for a player or Pal. It's indexed the same lazy way as any
//! other nested struct (`gvas::value::read_property_list`): spans only, decoded on
//! demand. Whatever a caller does with those spans, it must pass *this* struct's own
//! `bytes` slice back in as the source buffer — the spans are offsets into it, not
//! into the enclosing save file.

use super::error::RawDataError;
use crate::gvas::PropertyEntry;
use crate::gvas::primitives::{Guid, read_guid, read_u8, write_guid};
use crate::gvas::value::read_property_list;

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterData {
    /// The Pal/player's own properties (stats, level, IVs, passives, ...), indexed
    /// lazily. Materialize an entry with `gvas::value::materialize_property`, passing
    /// the same `bytes` slice this `CharacterData` was decoded from as `source`.
    pub object: Vec<PropertyEntry>,
    /// Byte offset (into the same `bytes` slice `object`'s spans point into) where
    /// the property list — including its "None" terminator, which isn't captured as
    /// an entry — ends. `encode()` uses this to replay the whole list verbatim
    /// instead of reconstructing the terminator's bytes from scratch.
    object_end: usize,
    pub unknown_bytes: [u8; 4],
    pub group_id: Guid,
    pub trailing_bytes: [u8; 4],
}

fn read_bytes_fixed<const N: usize>(buf: &[u8], pos: &mut usize) -> Result<[u8; N], RawDataError> {
    let mut out = [0u8; N];
    for slot in out.iter_mut() {
        *slot = read_u8(buf, pos)?;
    }
    Ok(out)
}

/// `has_property_guid` should come from the enclosing save's `Header`, same as for
/// the top-level GVAS property list — this nested one uses the same engine-version
/// gating.
pub fn decode(bytes: &[u8], has_property_guid: bool) -> Result<CharacterData, RawDataError> {
    let mut pos = 0usize;
    let object = read_property_list(bytes, &mut pos, has_property_guid)?;
    let object_end = pos;
    let unknown_bytes = read_bytes_fixed(bytes, &mut pos)?;
    let group_id = read_guid(bytes, &mut pos)?;
    let trailing_bytes = read_bytes_fixed(bytes, &mut pos)?;

    if pos != bytes.len() {
        return Err(RawDataError::NotExhausted {
            consumed: pos,
            total: bytes.len(),
        });
    }

    Ok(CharacterData {
        object,
        object_end,
        unknown_bytes,
        group_id,
        trailing_bytes,
    })
}

/// Re-emits the blob from `source`: `object`'s bytes (including the "None"
/// terminator) are replayed verbatim rather than re-derived from the indexed
/// entries, so the result is byte-identical to what was decoded.
///
/// This is **not** the path for editing a character's properties. `CharacterData`
/// holds spans, not values, so there is nothing here to mutate — an edit inside
/// `object` is applied by splicing the save directly with
/// `crate::edit::replace_property_value`, whose ancestor chain reaches the property
/// without this function being involved at all.
pub fn encode(source: &[u8], data: &CharacterData) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&source[..data.object_end]);
    out.extend_from_slice(&data.unknown_bytes);
    write_guid(&mut out, &data.group_id);
    out.extend_from_slice(&data.trailing_bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gvas::primitives::{FString, write_fstring, write_i32_le};
    use crate::gvas::property::{PropertyTag, TagExtra, none_terminator, write_property_tag};

    fn ascii(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    fn synthetic_bytes() -> Vec<u8> {
        let mut buf = Vec::new();
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
        write_fstring(&mut buf, &none_terminator());
        buf.extend_from_slice(&[1, 2, 3, 4]); // unknown_bytes
        crate::gvas::primitives::write_guid(&mut buf, &[7u8; 16]); // group_id
        buf.extend_from_slice(&[5, 6, 7, 8]); // trailing_bytes
        buf
    }

    #[test]
    fn decodes_and_indexes_the_nested_property_list() {
        let bytes = synthetic_bytes();
        let data = decode(&bytes, true).unwrap();
        assert_eq!(data.object.len(), 1);
        assert_eq!(data.object[0].name, "Level");
        assert_eq!(data.unknown_bytes, [1, 2, 3, 4]);
        assert_eq!(data.group_id, [7u8; 16]);
        assert_eq!(data.trailing_bytes, [5, 6, 7, 8]);
    }

    #[test]
    fn round_trips_including_the_none_terminator() {
        let bytes = synthetic_bytes();
        let data = decode(&bytes, true).unwrap();
        assert_eq!(encode(&bytes, &data), bytes);
    }

    #[test]
    fn empty_property_list_round_trips() {
        let mut buf = Vec::new();
        write_fstring(&mut buf, &none_terminator());
        buf.extend_from_slice(&[0, 0, 0, 0]);
        crate::gvas::primitives::write_guid(&mut buf, &[1u8; 16]);
        buf.extend_from_slice(&[0, 0, 0, 0]);

        let data = decode(&buf, true).unwrap();
        assert!(data.object.is_empty());
        assert_eq!(encode(&buf, &data), buf);
    }

    #[test]
    fn trailing_bytes_are_rejected_not_dropped() {
        let mut bytes = synthetic_bytes();
        bytes.push(0xFF);
        assert!(matches!(
            decode(&bytes, true),
            Err(RawDataError::NotExhausted { .. })
        ));
    }
}
