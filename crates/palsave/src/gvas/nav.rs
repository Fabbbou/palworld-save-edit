//! Navigating the lazy property tree without hand-rolling the same four things at
//! every call site.
//!
//! `materialize_property` needs `(source, engine_major, has_property_guid, path)`.
//! Three of those are fixed for a whole save, and the fourth is built by string
//! concatenation as you descend — so every caller was writing the same three lines
//! (find the entry by name, build the path, materialize) over and over.
//!
//! ## The trap this exists to remove
//!
//! `PropertyEntry.span` is an offset into *the buffer the entry was parsed from* —
//! which is not always the save. A Palworld `RawData` blob contains its own nested
//! GVAS property list, and those entries' spans are relative to **the blob**. Passing
//! the save buffer when materializing one of them reads a byte range from the wrong
//! place: no error, no panic, just wrong values, which is the exact failure class
//! `CLAUDE.md` is written to prevent. [`Cursor::rebase`] makes the re-rooting an
//! explicit, typed step, so the buffer and the entries that index it travel together.

use super::header::Header;
use super::primitives::Guid;
use super::value::{Value, materialize_property};
use super::{GvasError, PropertyEntry};

/// Finds a property by name in a materialized property list.
///
/// Palworld property lists are short (tens of entries) and the game gives fields
/// distinct names within a struct, so a linear scan is both correct and fast enough;
/// building a map per lookup would cost more than it saves.
pub fn find<'a>(props: &'a [PropertyEntry], name: &str) -> Option<&'a PropertyEntry> {
    props.iter().find(|p| p.name == name)
}

/// A buffer plus the save-wide parameters needed to decode properties indexed against
/// it. Cheap to clone; holds no decoded data.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    source: &'a [u8],
    engine_major: u16,
    has_property_guid: bool,
    path: String,
}

impl<'a> Cursor<'a> {
    /// A cursor over a whole save's GVAS buffer, rooted at the save root.
    pub fn new(source: &'a [u8], header: &Header) -> Self {
        Cursor {
            source,
            engine_major: header.engine_version_major,
            has_property_guid: header.has_property_guid(),
            path: String::new(),
        }
    }

    /// Builds a cursor from already-extracted parts. For callers that own a blob and
    /// its decode parameters separately and must rebuild the cursor on demand —
    /// a struct holding both the blob and a `Cursor` borrowing it would be
    /// self-referential, which Rust won't allow without `Pin` or unsafe.
    pub fn new_raw(
        source: &'a [u8],
        engine_major: u16,
        has_property_guid: bool,
        path: &str,
    ) -> Self {
        Cursor {
            source,
            engine_major,
            has_property_guid,
            path: path.to_string(),
        }
    }

    pub fn source(&self) -> &'a [u8] {
        self.source
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn engine_major(&self) -> u16 {
        self.engine_major
    }

    pub fn has_property_guid(&self) -> bool {
        self.has_property_guid
    }

    /// Re-roots onto a nested buffer — a `RawData` blob — carrying the save-wide
    /// decode parameters across. The returned cursor borrows `blob`, not the save, so
    /// the compiler stops you handing it entries indexed against the wrong buffer.
    pub fn rebase<'b>(&self, blob: &'b [u8], path: &str) -> Cursor<'b> {
        Cursor {
            source: blob,
            engine_major: self.engine_major,
            has_property_guid: self.has_property_guid,
            path: path.to_string(),
        }
    }

    /// The dotted path a child of this cursor would have. Exposed because the hint
    /// table in `gvas::hints` is keyed by these paths.
    pub fn child_path(&self, name: &str) -> String {
        if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}.{}", self.path, name)
        }
    }

    /// Materializes `entry`, which must be indexed against this cursor's buffer.
    pub fn materialize(&self, entry: &PropertyEntry) -> Result<Value, GvasError> {
        materialize_property(
            self.source,
            entry,
            self.engine_major,
            self.has_property_guid,
            &self.child_path(&entry.name),
        )
    }

    /// Finds a named property and materializes it. `Ok(None)` means "no such
    /// property", which is a normal outcome across game versions — distinct from
    /// `Err`, which means the bytes were there but wouldn't decode.
    pub fn get(&self, props: &[PropertyEntry], name: &str) -> Result<Option<Value>, GvasError> {
        match find(props, name) {
            Some(entry) => Ok(Some(self.materialize(entry)?)),
            None => Ok(None),
        }
    }

    /// `get`, discarding the distinction between "absent" and "failed to decode".
    /// For optional display fields where either way the answer is "we don't know".
    pub fn get_opt(&self, props: &[PropertyEntry], name: &str) -> Option<Value> {
        self.get(props, name).ok().flatten()
    }

    /// Descends into a named child struct, returning both its property list and a
    /// cursor scoped to it (same buffer, extended path).
    pub fn descend(
        &self,
        props: &[PropertyEntry],
        name: &str,
    ) -> Result<Option<(Vec<PropertyEntry>, Cursor<'a>)>, GvasError> {
        let Some(value) = self.get(props, name)? else {
            return Ok(None);
        };
        let Some(children) = value.as_properties() else {
            return Ok(None);
        };
        let cursor = Cursor {
            source: self.source,
            engine_major: self.engine_major,
            has_property_guid: self.has_property_guid,
            path: self.child_path(name),
        };
        Ok(Some((children.to_vec(), cursor)))
    }
}

/// A `Guid` as 32 lowercase hex chars, in **Unreal's own display convention**: four
/// little-endian `u32` groups, each printed big-endian.
///
/// This is not cosmetic. Palworld names each player's save file after their
/// `PlayerUId` in exactly this form, so a raw byte-order dump doesn't match: the
/// fixture's player reads as `…01000000` byte-wise but their file is
/// `00000000000000000000000000000001.sav`. Matching the game's convention is what
/// lets a uid shown in the UI be recognised, and what will let a future migration
/// feature pair a character map entry with its `Players/<uid>.sav`.
///
/// Deliberately undashed — this is a stable reversible handle, not an RFC-4122 UUID,
/// and Unreal's field order versus RFC-4122's is its own source of confusion.
pub fn guid_to_hex(guid: &Guid) -> String {
    let mut s = String::with_capacity(32);
    for group in guid.chunks_exact(4) {
        let value = u32::from_le_bytes([group[0], group[1], group[2], group[3]]);
        s.push_str(&format!("{value:08x}"));
    }
    s
}

/// Inverse of [`guid_to_hex`].
pub fn hex_to_guid(hex: &str) -> Option<Guid> {
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for group in 0..4 {
        let value = u32::from_str_radix(hex.get(group * 8..group * 8 + 8)?, 16).ok()?;
        out[group * 4..group * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> PropertyEntry {
        PropertyEntry {
            name: name.to_string(),
            type_name: "IntProperty".to_string(),
            span: 0..0,
        }
    }

    #[test]
    fn find_matches_by_name() {
        let props = vec![entry("A"), entry("B")];
        assert_eq!(find(&props, "B").map(|p| p.name.as_str()), Some("B"));
        assert!(find(&props, "missing").is_none());
    }

    #[test]
    fn child_path_builds_dotted_paths_from_the_root() {
        let header = Header {
            magic: 0,
            save_game_version: 3,
            package_version_ue4: 0,
            package_version_ue5: None,
            engine_version_major: 5,
            engine_version_minor: 1,
            engine_version_patch: 1,
            engine_version_build: 0,
            engine_version_branch: super::super::primitives::FString::Empty,
            custom_version: None,
        };
        let cursor = Cursor::new(&[], &header);
        // Root has no leading dot...
        assert_eq!(cursor.child_path("worldSaveData"), "worldSaveData");

        let nested = cursor.rebase(&[], "worldSaveData.GroupSaveDataMap");
        // ...and a rebased cursor extends whatever path it was given.
        assert_eq!(
            nested.child_path("RawData"),
            "worldSaveData.GroupSaveDataMap.RawData"
        );
    }

    #[test]
    fn rebase_carries_decode_parameters_but_swaps_the_buffer() {
        let header = Header {
            magic: 0,
            save_game_version: 3,
            package_version_ue4: 0,
            package_version_ue5: None,
            engine_version_major: 5,
            engine_version_minor: 1,
            engine_version_patch: 1,
            engine_version_build: 0,
            engine_version_branch: super::super::primitives::FString::Empty,
            custom_version: None,
        };
        let save = [1u8, 2, 3];
        let blob = [9u8, 9];
        let cursor = Cursor::new(&save, &header);
        let rebased = cursor.rebase(&blob, "x");

        assert_eq!(rebased.source(), &blob);
        assert_eq!(rebased.engine_major(), cursor.engine_major());
        assert_eq!(rebased.has_property_guid(), cursor.has_property_guid());
    }

    #[test]
    fn guid_hex_round_trips() {
        // Obviously-synthetic: test data, not a GUID lifted from a real save.
        // Each 4-byte group is reversed on display, per Unreal's convention.
        let guid: Guid = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let hex = guid_to_hex(&guid);
        assert_eq!(hex, "3322110077665544bbaa9988ffeeddcc");
        assert_eq!(hex_to_guid(&hex), Some(guid));
    }

    /// The convention is load-bearing: Palworld names each player's save file after
    /// their `PlayerUId` rendered this way. A raw byte-order dump would print
    /// `…01000000` and never match `00000000000000000000000000000001.sav`.
    #[test]
    fn guid_hex_matches_palworld_player_file_naming() {
        let mut player_uid: Guid = [0u8; 16];
        player_uid[12] = 0x01; // little-endian 1 in the final u32 group
        assert_eq!(guid_to_hex(&player_uid), "00000000000000000000000000000001");
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert_eq!(hex_to_guid("too short"), None);
        assert_eq!(hex_to_guid(&"z".repeat(32)), None);
    }
}
