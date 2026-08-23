//! Moving a player, their Pals and their belongings from one world into another.
//!
//! This module is the *survey*. It works out exactly which rows a migration would copy
//! and what already occupies those identities in the destination — and writes nothing.
//! Applying a plan is a separate step with its own gates.
//!
//! ## Why the survey is a separate, first-class thing
//!
//! `CLAUDE.md` is blunt about the stakes: a plausible-looking wrong answer here destroys
//! 200-hour worlds. A migration touches four maps and an array at once, and the failure
//! that matters is not a crash — it's a silent identity collision, where the destination
//! already has a player with that uid, or a Pal with that instance id, and the copy
//! quietly produces a world with two of something that must be unique.
//!
//! Those collisions are knowable before a single byte moves. So they are computed
//! first, reported as data, and handed to the caller to decide about. A migration that
//! can't be previewed shouldn't be run.
//!
//! ## What a migration consists of
//!
//! Six things travel together, and a partial copy is worse than none:
//!
//! | what | where it lives | how it's found |
//! |---|---|---|
//! | the player | `CharacterSaveParameterMap` | key `PlayerUId` == theirs |
//! | their Pals | same map | `OwnerPlayerUId` in each blob |
//! | item containers | `ItemContainerSaveData` | ids from their own save |
//! | Pal containers | `CharacterContainerSaveData` | ids from their own save |
//! | per-item state | `DynamicItemSaveData` | `DynamicId`s on their item slots |
//! | the player file | `Players/<uid>.sav` | copied wholesale |
//!
//! Container ids are read from the player's own save, never guessed from the container
//! side — the rule `inventory` already states, and it matters more here, because
//! guessing wrong would move somebody else's chest.
//!
//! ## Instance ids are not globally unique, and that decides the design
//!
//! A Pal's `InstanceId` looks like a random guid, which makes the obvious assumption
//! "copying a Pal between worlds cannot clash". **That assumption is false**, and it was
//! only caught because two unrelated real worlds were available to check against.
//!
//! `pal_instance_ids_collide_across_unrelated_worlds` finds three ids present in both
//! worlds in the fixture corpus. In every case the two Pals are the *same species* with
//! different levels and different owners — `SwordCutlassfish`, `Eagle`,
//! `BOSS_SheepBall`. The likely mechanism is that world-placed Pals take an id derived
//! from something deterministic, such as a spawn point on a map both worlds share, and
//! keep it once caught.
//!
//! Whatever the cause, the consequence is fixed: **a migration that copies rows
//! verbatim will eventually put two characters with one instance id into a world, and
//! nothing downstream would notice.** Detecting the clash is therefore not a nicety
//! bolted on to a copy — it is the reason the survey exists, and id remapping is a
//! requirement of the apply step rather than an option for it.
//!
//! The same goes for the player uid. Both worlds here contain a player
//! `00000000000000000000000000000001`, which is what an offline first player gets.
//! Different people, same identity. Migrating between two single-player worlds is
//! therefore the *typical* case, not an edge case, and it always collides.

use crate::gvas::GvasError;
use crate::gvas::nav::{guid_to_hex, hex_to_guid};
use crate::gvas::primitives::Guid;
use crate::inventory::{self, InventoryError};
use crate::rawdata::error::RawDataError;
use crate::rawdata::item_container::DynamicId;
use crate::rawdata::{character, dynamic_item, item_container};
use crate::world::{self, WorldError};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const CHARACTER_MAP: &str = "CharacterSaveParameterMap";

#[derive(Debug)]
pub enum MigrateError {
    World(WorldError),
    Gvas(GvasError),
    RawData(RawDataError),
    Inventory(InventoryError),
    /// The uid isn't a 32-hex-character string.
    MalformedUid,
    /// No player with that uid in the source world.
    PlayerNotFound {
        uid: String,
    },
}

impl MigrateError {
    pub fn code(&self) -> &'static str {
        match self {
            MigrateError::World(e) => e.code(),
            MigrateError::Gvas(_) => "gvas_parse_failed",
            MigrateError::RawData(_) => "rawdata_decode_failed",
            MigrateError::Inventory(e) => e.code(),
            MigrateError::MalformedUid => "malformed_uid",
            MigrateError::PlayerNotFound { .. } => "player_not_found",
        }
    }
}

impl From<WorldError> for MigrateError {
    fn from(e: WorldError) -> Self {
        MigrateError::World(e)
    }
}
impl From<GvasError> for MigrateError {
    fn from(e: GvasError) -> Self {
        MigrateError::Gvas(e)
    }
}
impl From<RawDataError> for MigrateError {
    fn from(e: RawDataError) -> Self {
        MigrateError::RawData(e)
    }
}
impl From<InventoryError> for MigrateError {
    fn from(e: InventoryError) -> Self {
        MigrateError::Inventory(e)
    }
}

impl fmt::Display for MigrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MigrateError::World(e) => write!(f, "{e}"),
            MigrateError::Gvas(e) => write!(f, "{e}"),
            MigrateError::RawData(e) => write!(f, "{e}"),
            MigrateError::Inventory(e) => write!(f, "{e}"),
            MigrateError::MalformedUid => write!(f, "uid must be 32 hex characters"),
            MigrateError::PlayerNotFound { uid } => {
                write!(f, "no player {uid} in the source world")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

/// An identity that already exists in the destination.
///
/// Every variant means "copying this row would produce two things sharing one
/// identity". None of them is automatically fatal — replacing your own character in a
/// world you also play in is a legitimate thing to want — but none may be decided
/// silently, so they are reported rather than resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// The destination already has this player. The common case when moving between two
    /// worlds you play in, and the one that must never be resolved by guessing.
    PlayerExists { uid: String },
    /// A Pal with this instance id is already in the destination.
    PalInstanceExists { instance_id: String },
    /// A container with this id already exists there.
    ContainerExists { id: String },
    /// A dynamic item row with this id already exists there.
    DynamicItemExists { id: String },
    /// The player's character blob names a guild that the destination has no entry for.
    /// Not an identity clash — the opposite: something they reference goes missing.
    GuildMissing { group_id: String },
}

impl Conflict {
    /// Stable identifier for the wasm boundary and the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Conflict::PlayerExists { .. } => "player_exists",
            Conflict::PalInstanceExists { .. } => "pal_instance_exists",
            Conflict::ContainerExists { .. } => "container_exists",
            Conflict::DynamicItemExists { .. } => "dynamic_item_exists",
            Conflict::GuildMissing { .. } => "guild_missing",
        }
    }
}

/// What a migration would move, and what stands in its way. Counts and ids only — the
/// bytes are re-read at apply time from the source, so a plan stays small enough to
/// cross the wasm boundary and can't go stale silently.
#[derive(Debug, Clone, PartialEq)]
pub struct MigrationPlan {
    pub player_uid: String,
    /// Index into the source `CharacterSaveParameterMap`'s wire layout.
    pub player_entry_index: usize,
    pub pal_entry_indices: Vec<usize>,
    pub item_container_indices: Vec<usize>,
    pub pal_container_indices: Vec<usize>,
    pub dynamic_item_indices: Vec<usize>,
    /// The guild the player belongs to in the source world, if any.
    pub source_group_id: Option<String>,
    pub conflicts: Vec<Conflict>,
}

impl MigrationPlan {
    /// Total rows that would be written. Useful as a sanity figure in a UI: a migration
    /// that claims to move two rows is not moving a player.
    pub fn row_count(&self) -> usize {
        1 + self.pal_entry_indices.len()
            + self.item_container_indices.len()
            + self.pal_container_indices.len()
            + self.dynamic_item_indices.len()
    }

    /// Conflicts that would leave the destination with two of something unique.
    /// `GuildMissing` is excluded — it's a dangling reference, not a duplicate.
    pub fn blocking_conflicts(&self) -> impl Iterator<Item = &Conflict> {
        self.conflicts
            .iter()
            .filter(|c| !matches!(c, Conflict::GuildMissing { .. }))
    }
}

/// One character map entry, reduced to the identity fields a migration reasons about.
struct CharacterRow {
    index: usize,
    player_uid: Option<String>,
    instance_id: String,
    owner_uid: Option<String>,
    group_id: Option<String>,
    is_player: bool,
}

/// Reads every character entry's identity, paired with its position in the map's wire
/// layout.
///
/// `characters::list_all_pals` already decodes these blobs, but it discards the map
/// index — and an index is exactly what a splice needs. Rather than widen that API for
/// one caller, this walks the same map and keeps the position.
fn character_rows(gvas: &[u8]) -> Result<Vec<CharacterRow>, MigrateError> {
    let map = world::open_map(gvas, CHARACTER_MAP)?;
    let value_path = format!("worldSaveData.{CHARACTER_MAP}.Value");
    let mut out = Vec::with_capacity(map.entries.len());

    for (index, entry) in map.entries.iter().enumerate() {
        let (player_uid, instance_id) = match entry.key.as_properties() {
            Some(key_props) => (
                map.cursor
                    .get_opt(key_props, "PlayerUId")
                    .and_then(|v| v.as_guid())
                    .map(|g| guid_to_hex(&g)),
                map.cursor
                    .get_opt(key_props, "InstanceId")
                    .and_then(|v| v.as_guid())
                    .map(|g| guid_to_hex(&g)),
            ),
            None => (None, None),
        };

        let Some(raw_entry) = crate::gvas::nav::find(&entry.fields, "RawData") else {
            continue;
        };
        let Some(blob) = map
            .cursor
            .materialize(raw_entry)?
            .as_bytes()
            .map(|b| b.to_vec())
        else {
            continue;
        };
        let decoded = character::decode(&blob, map.cursor.has_property_guid())?;

        let blob_cursor = map.cursor.rebase(&blob, &format!("{value_path}.RawData"));
        let save_parameter = blob_cursor
            .get_opt(&decoded.object, "SaveParameter")
            .and_then(|v| v.as_properties().map(|p| p.to_vec()))
            .unwrap_or_default();
        let is_player = blob_cursor
            .get_opt(&save_parameter, "IsPlayer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let owner_uid = blob_cursor
            .get_opt(&save_parameter, "OwnerPlayerUId")
            .and_then(|v| v.as_guid())
            .map(|g| guid_to_hex(&g))
            .filter(|u| u.chars().any(|c| c != '0'));

        out.push(CharacterRow {
            index,
            // An all-zero PlayerUId is how the format says "this is a Pal", not a
            // missing value.
            player_uid: player_uid.filter(|u| u.chars().any(|c| c != '0')),
            instance_id: instance_id.unwrap_or_default(),
            owner_uid,
            group_id: Some(guid_to_hex(&decoded.group_id)).filter(|g| g.chars().any(|c| c != '0')),
            is_player,
        });
    }
    Ok(out)
}

/// `container guid -> index` over a container map's wire layout.
fn container_index(gvas: &[u8], map_name: &str) -> Result<BTreeMap<Guid, usize>, MigrateError> {
    let map = world::open_map(gvas, map_name)?;
    let mut out = BTreeMap::new();
    for (index, entry) in map.entries.iter().enumerate() {
        if let Some(key_props) = entry.key.as_properties()
            && let Some(id) = map
                .cursor
                .get_opt(key_props, "ID")
                .and_then(|v| v.as_guid())
        {
            out.insert(id, index);
        }
    }
    Ok(out)
}

/// `DynamicId -> index` over `DynamicItemSaveData`'s wire layout.
fn dynamic_index(gvas: &[u8]) -> Result<BTreeMap<DynamicId, usize>, MigrateError> {
    let array = world::open_array(gvas, inventory::DYNAMIC_ITEM_ARRAY)?;
    let mut out = BTreeMap::new();
    for (index, fields) in array.elements.iter().enumerate() {
        let Some(raw) = array
            .cursor
            .get_opt(fields, "RawData")
            .and_then(|v| v.as_bytes().map(|b| b.to_vec()))
        else {
            continue;
        };
        if let Ok(decoded) = dynamic_item::decode(&raw) {
            out.insert(decoded.id, index);
        }
    }
    Ok(out)
}

/// Every `DynamicId` referenced by the occupied slots of the given containers.
fn dynamic_ids_used_by(
    gvas: &[u8],
    map_name: &str,
    wanted: &BTreeSet<usize>,
) -> Result<BTreeSet<DynamicId>, MigrateError> {
    let map = world::open_map(gvas, map_name)?;
    let mut out = BTreeSet::new();
    for (index, entry) in map.entries.iter().enumerate() {
        if !wanted.contains(&index) {
            continue;
        }
        let Some(slots) = map
            .cursor
            .get_opt(&entry.fields, "Slots")
            .and_then(|v| v.as_array().map(|a| a.to_vec()))
        else {
            continue;
        };
        for slot in &slots {
            let Some(fields) = slot.as_properties() else {
                continue;
            };
            let Some(raw) = map
                .cursor
                .get_opt(fields, "RawData")
                .and_then(|v| v.as_bytes().map(|b| b.to_vec()))
            else {
                continue;
            };
            let Ok(decoded) = item_container::decode_slot(&raw) else {
                continue;
            };
            if decoded.count > 0 && !decoded.item.dynamic_id.is_zero() {
                out.insert(decoded.item.dynamic_id);
            }
        }
    }
    Ok(out)
}

/// Surveys what moving `player_uid` from `source_level` into `target_level` would
/// involve. Reads three files and writes none.
///
/// `source_player` is the player's own `Players/<uid>.sav` from the **source** world,
/// which is where their container ids live.
pub fn plan(
    source_level: &[u8],
    source_player: &[u8],
    target_level: &[u8],
    player_uid: &str,
) -> Result<MigrationPlan, MigrateError> {
    if hex_to_guid(player_uid).is_none() {
        return Err(MigrateError::MalformedUid);
    }
    let uid = player_uid.to_lowercase();

    // --- what the source has -------------------------------------------------
    let source_rows = character_rows(source_level)?;
    let player = source_rows
        .iter()
        .find(|r| r.is_player && r.player_uid.as_deref() == Some(uid.as_str()))
        .ok_or_else(|| MigrateError::PlayerNotFound { uid: uid.clone() })?;

    let pals: Vec<&CharacterRow> = source_rows
        .iter()
        .filter(|r| !r.is_player && r.owner_uid.as_deref() == Some(uid.as_str()))
        .collect();

    // Container ids come from the player's own file. Never from the container side.
    let item_ids = inventory::container_ids(source_player)?;
    let pal_ids = inventory::pal_container_ids(source_player)?;

    let source_item_index = container_index(source_level, inventory::ITEM_CONTAINER_MAP)?;
    let source_pal_index = container_index(source_level, inventory::PAL_CONTAINER_MAP)?;

    let item_container_indices: Vec<usize> = item_ids
        .values()
        .filter_map(|g| source_item_index.get(g).copied())
        .collect();
    let pal_container_indices: Vec<usize> = pal_ids
        .values()
        .filter_map(|g| source_pal_index.get(g).copied())
        .collect();

    let used_dynamic = dynamic_ids_used_by(
        source_level,
        inventory::ITEM_CONTAINER_MAP,
        &item_container_indices.iter().copied().collect(),
    )?;
    let source_dynamic = dynamic_index(source_level)?;
    let dynamic_item_indices: Vec<usize> = used_dynamic
        .iter()
        .filter_map(|id| source_dynamic.get(id).copied())
        .collect();

    // --- what the target already holds ---------------------------------------
    let target_rows = character_rows(target_level)?;
    let target_players: BTreeSet<&str> = target_rows
        .iter()
        .filter(|r| r.is_player)
        .filter_map(|r| r.player_uid.as_deref())
        .collect();
    let target_instances: BTreeSet<&str> =
        target_rows.iter().map(|r| r.instance_id.as_str()).collect();
    let target_groups: BTreeSet<String> = {
        let map = world::open_map(target_level, "GroupSaveDataMap")?;
        map.entries
            .iter()
            .filter_map(|e| e.key.as_guid().map(|g| guid_to_hex(&g)))
            .collect()
    };

    let target_item_index = container_index(target_level, inventory::ITEM_CONTAINER_MAP)?;
    let target_pal_index = container_index(target_level, inventory::PAL_CONTAINER_MAP)?;
    let target_dynamic = dynamic_index(target_level)?;

    let mut conflicts = Vec::new();
    if target_players.contains(uid.as_str()) {
        conflicts.push(Conflict::PlayerExists { uid: uid.clone() });
    }
    for pal in &pals {
        if target_instances.contains(pal.instance_id.as_str()) {
            conflicts.push(Conflict::PalInstanceExists {
                instance_id: pal.instance_id.clone(),
            });
        }
    }
    for guid in item_ids.values().chain(pal_ids.values()) {
        if target_item_index.contains_key(guid) || target_pal_index.contains_key(guid) {
            conflicts.push(Conflict::ContainerExists {
                id: guid_to_hex(guid),
            });
        }
    }
    for id in &used_dynamic {
        if target_dynamic.contains_key(id) {
            conflicts.push(Conflict::DynamicItemExists {
                id: guid_to_hex(&id.local_id_in_created_world),
            });
        }
    }
    if let Some(group) = &player.group_id
        && !target_groups.contains(group)
    {
        conflicts.push(Conflict::GuildMissing {
            group_id: group.clone(),
        });
    }

    Ok(MigrationPlan {
        player_uid: uid,
        player_entry_index: player.index,
        pal_entry_indices: pals.iter().map(|p| p.index).collect(),
        item_container_indices,
        pal_container_indices,
        dynamic_item_indices,
        source_group_id: player.group_id.clone(),
        conflicts,
    })
}
