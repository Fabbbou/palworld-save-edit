//! The one descent every `Level.sav` task shares: reach a named map under
//! `worldSaveData` and hand back its entries.
//!
//! `GroupSaveDataMap`, `CharacterSaveParameterMap`, `ItemContainerSaveData` and the
//! rest all sit at `worldSaveData.<Name>` and are all `MapProperty`. Before this
//! existed, `guilds.rs` and `examples/peek_character.rs` each carried their own
//! byte-identical copy of the walk.
//!
//! [`WorldMap`] deliberately keeps `world_entry` and `map_entry`: those two are
//! exactly the ancestor chain `crate::edit::replace_property_value` needs to fix up
//! enclosing `size` fields, so anything built on this gets edit support for free
//! rather than having to re-derive the chain later.

use crate::gvas::nav::{Cursor, find};
use crate::gvas::value::Value;
use crate::gvas::{GvasError, GvasFile, PropertyEntry};
use std::fmt;

#[derive(Debug)]
pub enum WorldError {
    Gvas(GvasError),
    /// No `worldSaveData` property — this isn't a `Level.sav` (a `Players/*.sav` or
    /// `LevelMeta.sav` will land here, which is expected, not a malformed file).
    NotALevelSave,
    /// `worldSaveData` exists but has no map by that name, or it didn't decode as a
    /// map. Carries the name so the caller can report which one.
    MapNotFound {
        name: String,
    },
}

impl WorldError {
    pub fn code(&self) -> &'static str {
        match self {
            WorldError::Gvas(_) => "gvas_parse_failed",
            WorldError::NotALevelSave => "not_a_level_save",
            WorldError::MapNotFound { .. } => "map_not_found",
        }
    }
}

impl From<GvasError> for WorldError {
    fn from(e: GvasError) -> Self {
        WorldError::Gvas(e)
    }
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldError::Gvas(e) => write!(f, "{e}"),
            WorldError::NotALevelSave => {
                write!(f, "this save has no worldSaveData (not a Level.sav)")
            }
            WorldError::MapNotFound { name } => {
                write!(f, "worldSaveData has no map named {name}")
            }
        }
    }
}

impl std::error::Error for WorldError {}

/// One entry of a `worldSaveData` map: the materialized key, and the value's property
/// list. Values in these maps are always generic structs (a property list) — the
/// per-path hints in `gvas::hints` are what make that decode correctly.
pub struct MapEntry {
    pub key: Value,
    pub fields: Vec<PropertyEntry>,
}

pub struct WorldMap<'a> {
    /// The top-level `worldSaveData` property. Ancestor chain, element 0.
    pub world_entry: PropertyEntry,
    /// The named map property. Ancestor chain, element 1.
    pub map_entry: PropertyEntry,
    pub entries: Vec<MapEntry>,
    /// Scoped to `worldSaveData.<Name>`, so `cursor.child_path("Value")` and friends
    /// line up with the hint table.
    pub cursor: Cursor<'a>,
}

/// Walks `worldSaveData.<map_name>` and materializes its entries.
///
/// Entries whose value isn't a property list are skipped rather than erroring: a map
/// that partly fails to decode still yields the parts that worked, which matches the
/// fail-soft posture of `gvas::value` (unknown regions degrade to `Value::Raw`).
pub fn open_map<'a>(gvas: &'a [u8], map_name: &str) -> Result<WorldMap<'a>, WorldError> {
    let file = GvasFile::parse(gvas)?;
    let root = Cursor::new(gvas, &file.header);

    let world_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .ok_or(WorldError::NotALevelSave)?;
    let world_entry = file.properties[world_idx].clone();

    let world_value = file.materialize(world_idx)?;
    let world_children = world_value
        .as_properties()
        .ok_or(WorldError::NotALevelSave)?;

    let map_entry = find(world_children, map_name)
        .ok_or_else(|| WorldError::MapNotFound {
            name: map_name.to_string(),
        })?
        .clone();

    let world_cursor = Cursor::new(gvas, &file.header).rebase(gvas, "worldSaveData");
    let map_value = world_cursor.materialize(&map_entry)?;
    let raw_entries = map_value.as_map().ok_or_else(|| WorldError::MapNotFound {
        name: map_name.to_string(),
    })?;

    let entries = raw_entries
        .iter()
        .filter_map(|(key, value)| {
            value.as_properties().map(|fields| MapEntry {
                key: key.clone(),
                fields: fields.to_vec(),
            })
        })
        .collect();

    let cursor = root.rebase(gvas, &format!("worldSaveData.{map_name}"));
    Ok(WorldMap {
        world_entry,
        map_entry,
        entries,
        cursor,
    })
}

impl WorldMap<'_> {
    /// The ancestor chain for splicing a property inside one of this map's entries:
    /// `[worldSaveData, <map>, <leaf>]`. See `crate::edit::replace_property_value`.
    pub fn edit_chain<'e>(&'e self, leaf: &'e PropertyEntry) -> [&'e PropertyEntry; 3] {
        [&self.world_entry, &self.map_entry, leaf]
    }
}
