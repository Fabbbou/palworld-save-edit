//! A player's inventories, joined across two save files.
//!
//! `Level.sav` holds every item container in the world — 1488 of them in the
//! reference fixture — but says nothing about which belongs to whom. The mapping
//! lives in the player's own `Players/<uid>.sav`, under `SaveData.InventoryInfo`,
//! where each of the six container kinds is a struct wrapping an `ID` guid. That guid
//! is exactly the key of the `worldSaveData.ItemContainerSaveData` map.
//!
//! So resolving an inventory needs both files, and the direction matters: ids are
//! read from *the player's own* file and then looked up, never guessed from the
//! container side. The maintained reference (`oMaN-Rod/palworld-save-pal`,
//! `psp-core/src/domain/player.rs` -> `containers.rs`) is explicit that a
//! caller-supplied container id must never be trusted for this, and the same holds
//! here: it would let one player's edit land in another's chest.
//!
//! The same two-file join resolves a player's **Pals** — see [`player_pal_storage`] —
//! through a different map and one step deeper.
//!
//! Read-only. Editing items is a separate change with its own gates.

use crate::characters::{self, CharacterError, PalSummary};
use crate::gvas::nav::{self, Cursor};
use crate::gvas::primitives::Guid;
use crate::gvas::{GvasError, GvasFile};
use crate::rawdata::error::RawDataError;
use crate::rawdata::{character_container, item_container};
use crate::world::{self, WorldError};
use std::collections::BTreeMap;
use std::fmt;

pub const ITEM_CONTAINER_MAP: &str = "ItemContainerSaveData";
pub const PAL_CONTAINER_MAP: &str = "CharacterContainerSaveData";

/// The container kinds reachable from a player's `SaveData`, in the order a UI
/// should show them. The `&str` is the property name under `InventoryInfo`.
///
/// `PalStorageContainerId` and `OtomoCharacterContainerId` are deliberately absent:
/// they live at `SaveData` top level rather than under `InventoryInfo`, and point into
/// `CharacterContainerSaveData` — a different map, holding Pals rather than items.
/// They have their own table, [`PAL_CONTAINER_KINDS`].
pub const CONTAINER_KINDS: &[(ContainerKind, &str)] = &[
    (ContainerKind::Common, "CommonContainerId"),
    (ContainerKind::Essential, "EssentialContainerId"),
    (ContainerKind::Weapon, "WeaponLoadOutContainerId"),
    (ContainerKind::Armor, "PlayerEquipArmorContainerId"),
    (ContainerKind::Food, "FoodEquipContainerId"),
    (ContainerKind::DropSlot, "DropSlotContainerId"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Common,
    Essential,
    Weapon,
    Armor,
    Food,
    DropSlot,
}

impl ContainerKind {
    /// Stable identifier for the wasm boundary and the UI. Not localized.
    pub fn as_str(self) -> &'static str {
        match self {
            ContainerKind::Common => "common",
            ContainerKind::Essential => "essential",
            ContainerKind::Weapon => "weapon",
            ContainerKind::Armor => "armor",
            ContainerKind::Food => "food",
            ContainerKind::DropSlot => "drop_slot",
        }
    }
}

/// The Pal containers reachable from a player's `SaveData`, in the order a UI should
/// show them. Unlike [`CONTAINER_KINDS`] these sit at `SaveData` top level, not under
/// `InventoryInfo`.
///
/// A world's `CharacterContainerSaveData` holds more containers than these two — base
/// camps and the viewing cage have their own, 9 entries for 2 players in the reference
/// corpus. Only the ones a player's save actually names are resolvable *to that
/// player*, and guessing ownership from the container side is the mistake this module
/// exists to avoid.
pub const PAL_CONTAINER_KINDS: &[(PalContainerKind, &str)] = &[
    (PalContainerKind::Party, "OtomoCharacterContainerId"),
    (PalContainerKind::Storage, "PalStorageContainerId"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalContainerKind {
    /// The five Pals that follow the player.
    Party,
    /// The Pal box.
    Storage,
}

impl PalContainerKind {
    /// Stable identifier for the wasm boundary and the UI. Not localized.
    pub fn as_str(self) -> &'static str {
        match self {
            PalContainerKind::Party => "party",
            PalContainerKind::Storage => "storage",
        }
    }
}

#[derive(Debug)]
pub enum InventoryError {
    World(WorldError),
    Gvas(GvasError),
    RawData(RawDataError),
    /// Reading Pal-box contents needs `CharacterSaveParameterMap` decoded too, to turn
    /// a slot's instance id into a species and a level.
    Character(CharacterError),
    /// The file handed in as a player save has no `SaveData.InventoryInfo` — almost
    /// always because it's a `Level.sav`, not a player file.
    NotAPlayerSave,
}

impl InventoryError {
    pub fn code(&self) -> &'static str {
        match self {
            InventoryError::World(e) => e.code(),
            InventoryError::Gvas(_) => "gvas_parse_failed",
            InventoryError::RawData(_) => "rawdata_decode_failed",
            InventoryError::Character(e) => e.code(),
            InventoryError::NotAPlayerSave => "not_a_player_save",
        }
    }
}

impl From<WorldError> for InventoryError {
    fn from(e: WorldError) -> Self {
        InventoryError::World(e)
    }
}
impl From<GvasError> for InventoryError {
    fn from(e: GvasError) -> Self {
        InventoryError::Gvas(e)
    }
}
impl From<RawDataError> for InventoryError {
    fn from(e: RawDataError) -> Self {
        InventoryError::RawData(e)
    }
}
impl From<CharacterError> for InventoryError {
    fn from(e: CharacterError) -> Self {
        InventoryError::Character(e)
    }
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InventoryError::World(e) => write!(f, "{e}"),
            InventoryError::Gvas(e) => write!(f, "{e}"),
            InventoryError::RawData(e) => write!(f, "{e}"),
            InventoryError::Character(e) => write!(f, "{e}"),
            InventoryError::NotAPlayerSave => {
                write!(
                    f,
                    "this save has no SaveData.InventoryInfo (not a player save)"
                )
            }
        }
    }
}

impl std::error::Error for InventoryError {}

#[derive(Debug, Clone, PartialEq)]
pub struct SlotView {
    pub slot_index: i32,
    pub count: i32,
    /// Item id as the game stores it, e.g. `Wood`. There is no display-name table in
    /// this project, so this is what the UI shows.
    pub static_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContainerView {
    pub kind: ContainerKind,
    pub id: String,
    /// Declared capacity (`SlotNum`), which is larger than `slots.len()` because
    /// empty slots aren't listed.
    pub slot_count: i32,
    /// Occupied slots only, in slot order.
    pub slots: Vec<SlotView>,
    /// The player's file named a container id that no entry in `Level.sav` matches.
    /// Normal for a character that has never had, say, a drop slot — reported rather
    /// than hidden, because it's also what a mismatched pair of files looks like.
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInventory {
    pub player_uid: String,
    pub containers: Vec<ContainerView>,
}

/// One occupied Pal-container slot, joined to the Pal that sits in it.
#[derive(Debug, Clone, PartialEq)]
pub struct PalSlotView {
    /// Position in the container's `Slots` array. The blob carries no index of its own
    /// — see `rawdata::character_container` — so this is the position, and occupied
    /// slots are contiguous from 0.
    pub slot_index: i32,
    pub instance_id: String,
    /// The Pal, when `CharacterSaveParameterMap` has an entry for `instance_id`.
    /// `None` means the container references a Pal the world doesn't contain, which is
    /// a real (if rare) state in a damaged save and is shown rather than hidden.
    pub pal: Option<PalSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PalContainerView {
    pub kind: PalContainerKind,
    pub id: String,
    /// Declared capacity (`SlotNum`) — 960 for a Pal box, 5 for a party. Larger than
    /// `slots.len()`, which counts only occupied slots.
    pub slot_count: i32,
    pub slots: Vec<PalSlotView>,
    /// The player's file named a container id that no entry in `Level.sav` matches.
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPalStorage {
    pub player_uid: String,
    pub containers: Vec<PalContainerView>,
}

/// Reads `SaveData` out of a player save, erroring if this isn't one.
fn player_save_data<'a>(
    player_gvas: &'a [u8],
    file: &GvasFile<'a>,
) -> Result<(Vec<crate::gvas::PropertyEntry>, Cursor<'a>), InventoryError> {
    let idx = file
        .properties
        .iter()
        .position(|p| p.name == "SaveData")
        .ok_or(InventoryError::NotAPlayerSave)?;
    let value = file.materialize(idx)?;
    let props = value
        .as_properties()
        .ok_or(InventoryError::NotAPlayerSave)?
        .to_vec();
    let cursor = Cursor::new(player_gvas, &file.header).rebase(player_gvas, "SaveData");
    Ok((props, cursor))
}

/// A `<Name>ContainerId` property is a struct wrapping a single `ID` guid.
fn container_id(
    cursor: &Cursor<'_>,
    props: &[crate::gvas::PropertyEntry],
    name: &str,
) -> Option<Guid> {
    let value = cursor.get_opt(props, name)?;
    let inner = value.as_properties()?;
    cursor.get_opt(inner, "ID")?.as_guid()
}

/// The player's container ids, by kind. A kind whose property is absent is simply
/// not in the map — that's a save-version difference, not a failure.
pub fn container_ids(player_gvas: &[u8]) -> Result<BTreeMap<&'static str, Guid>, InventoryError> {
    let file = GvasFile::parse(player_gvas)?;
    let (save_data, cursor) = player_save_data(player_gvas, &file)?;

    let inventory_info = cursor
        .get_opt(&save_data, "InventoryInfo")
        .ok_or(InventoryError::NotAPlayerSave)?;
    let info_props = inventory_info
        .as_properties()
        .ok_or(InventoryError::NotAPlayerSave)?;
    let info_cursor = cursor.rebase(player_gvas, "SaveData.InventoryInfo");

    let mut out = BTreeMap::new();
    for (kind, property) in CONTAINER_KINDS {
        if let Some(guid) = container_id(&info_cursor, info_props, property) {
            out.insert(kind.as_str(), guid);
        }
    }
    Ok(out)
}

/// The player's Pal-container ids, by kind.
///
/// These read from `SaveData` directly, not from `SaveData.InventoryInfo` — see
/// [`PAL_CONTAINER_KINDS`].
pub fn pal_container_ids(
    player_gvas: &[u8],
) -> Result<BTreeMap<&'static str, Guid>, InventoryError> {
    let file = GvasFile::parse(player_gvas)?;
    let (save_data, cursor) = player_save_data(player_gvas, &file)?;

    let mut out = BTreeMap::new();
    for (kind, property) in PAL_CONTAINER_KINDS {
        if let Some(guid) = container_id(&cursor, &save_data, property) {
            out.insert(kind.as_str(), guid);
        }
    }
    Ok(out)
}

/// The player's own uid, from their save file rather than from a caller.
pub fn player_uid(player_gvas: &[u8]) -> Result<String, InventoryError> {
    let file = GvasFile::parse(player_gvas)?;
    let (save_data, cursor) = player_save_data(player_gvas, &file)?;
    cursor
        .get_opt(&save_data, "PlayerUId")
        .and_then(|v| v.as_guid())
        .map(|g| nav::guid_to_hex(&g))
        .ok_or(InventoryError::NotAPlayerSave)
}

/// Builds `container guid -> position in map.entries` in one pass.
///
/// Both container maps are keyed by a `{ID: Guid}` struct, so this serves item and Pal
/// containers alike. The index exists so only the handful of containers a player
/// actually names get decoded — walking all 1488 of them to find six would be pure
/// waste on an 8.5 MB save.
fn index_by_container_id(map: &world::WorldMap<'_>) -> BTreeMap<Guid, usize> {
    let mut index = BTreeMap::new();
    for (position, entry) in map.entries.iter().enumerate() {
        if let Some(key_props) = entry.key.as_properties()
            && let Some(id) = map
                .cursor
                .get_opt(key_props, "ID")
                .and_then(|v| v.as_guid())
        {
            index.insert(id, position);
        }
    }
    index
}

/// Joins a player's container ids against `Level.sav`'s item containers.
pub fn player_inventory(
    level_gvas: &[u8],
    player_gvas: &[u8],
) -> Result<PlayerInventory, InventoryError> {
    let uid = player_uid(player_gvas)?;
    let wanted = container_ids(player_gvas)?;

    let map = world::open_map(level_gvas, ITEM_CONTAINER_MAP)?;
    let index = index_by_container_id(&map);

    let mut containers = Vec::with_capacity(CONTAINER_KINDS.len());

    for (kind, _) in CONTAINER_KINDS {
        let Some(guid) = wanted.get(kind.as_str()) else {
            continue;
        };
        let id_hex = nav::guid_to_hex(guid);

        let Some(&position) = index.get(guid) else {
            containers.push(ContainerView {
                kind: *kind,
                id: id_hex,
                slot_count: 0,
                slots: Vec::new(),
                missing: true,
            });
            continue;
        };

        let entry = &map.entries[position];
        let slot_count = map
            .cursor
            .get_opt(&entry.fields, "SlotNum")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;

        let mut slots = Vec::new();
        if let Some(slot_values) = map
            .cursor
            .get_opt(&entry.fields, "Slots")
            .and_then(|v| v.as_array().map(|a| a.to_vec()))
        {
            for slot_value in &slot_values {
                let Some(slot_fields) = slot_value.as_properties() else {
                    continue;
                };
                let Some(raw) = map
                    .cursor
                    .get_opt(slot_fields, "RawData")
                    .and_then(|v| v.as_bytes().map(|b| b.to_vec()))
                else {
                    continue;
                };
                // A slot whose RawData won't decode is skipped rather than failing the
                // whole inventory — one bad slot shouldn't blank the screen.
                let Ok(decoded) = item_container::decode_slot(&raw) else {
                    continue;
                };
                // Empty slots are stored but carry no item; don't list them.
                if decoded.count <= 0 {
                    continue;
                }
                let static_id = decoded.item.static_id.display_lossy();
                slots.push(SlotView {
                    slot_index: decoded.slot_index,
                    count: decoded.count,
                    static_id: (!static_id.is_empty()).then_some(static_id),
                });
            }
        }
        slots.sort_by_key(|s| s.slot_index);
        containers.push(ContainerView {
            kind: *kind,
            id: id_hex,
            slot_count,
            slots,
            missing: false,
        });
    }

    Ok(PlayerInventory {
        player_uid: uid,
        containers,
    })
}

/// Joins a player's Pal-container ids against `Level.sav`, then joins each slot's
/// instance id against the character map to say *which Pal* is in it.
///
/// Three files' worth of indirection, one step deeper than [`player_inventory`]:
///
/// ```text
/// Players/<uid>.sav  SaveData.PalStorageContainerId.ID  -> Guid
///   -> worldSaveData.CharacterContainerSaveData[Guid].Slots[].RawData.instance_id
///     -> worldSaveData.CharacterSaveParameterMap        -> species, level, IVs
/// ```
///
/// The last hop goes through [`characters::list_all_pals`] rather than re-decoding
/// character blobs here. That costs one extra walk of the character map, and buys the
/// guarantee that this screen and the Players screen can never disagree about a Pal —
/// they are reading the same decoder.
pub fn player_pal_storage(
    level_gvas: &[u8],
    player_gvas: &[u8],
) -> Result<PlayerPalStorage, InventoryError> {
    let uid = player_uid(player_gvas)?;
    let wanted = pal_container_ids(player_gvas)?;

    let map = world::open_map(level_gvas, PAL_CONTAINER_MAP)?;
    let index = index_by_container_id(&map);

    let pals: BTreeMap<String, PalSummary> = characters::list_all_pals(level_gvas)?
        .into_iter()
        .map(|p| (p.instance_id.clone(), p))
        .collect();

    let mut containers = Vec::with_capacity(PAL_CONTAINER_KINDS.len());

    for (kind, _) in PAL_CONTAINER_KINDS {
        let Some(guid) = wanted.get(kind.as_str()) else {
            continue;
        };
        let id_hex = nav::guid_to_hex(guid);

        let Some(&position) = index.get(guid) else {
            containers.push(PalContainerView {
                kind: *kind,
                id: id_hex,
                slot_count: 0,
                slots: Vec::new(),
                missing: true,
            });
            continue;
        };

        let entry = &map.entries[position];
        let slot_count = map
            .cursor
            .get_opt(&entry.fields, "SlotNum")
            .and_then(|v| v.as_integer())
            .unwrap_or(0) as i32;

        let mut slots = Vec::new();
        if let Some(slot_values) = map
            .cursor
            .get_opt(&entry.fields, "Slots")
            .and_then(|v| v.as_array().map(|a| a.to_vec()))
        {
            for (position, slot_value) in slot_values.iter().enumerate() {
                let Some(slot_fields) = slot_value.as_properties() else {
                    continue;
                };
                let Some(raw) = map
                    .cursor
                    .get_opt(slot_fields, "RawData")
                    .and_then(|v| v.as_bytes().map(|b| b.to_vec()))
                else {
                    continue;
                };
                // One undecodable slot shouldn't blank the whole box — same posture as
                // the item join above.
                let Ok(decoded) = character_container::decode_slot(&raw) else {
                    continue;
                };
                let instance_id = nav::guid_to_hex(&decoded.instance_id);
                let pal = pals.get(&instance_id).cloned();
                slots.push(PalSlotView {
                    slot_index: position as i32,
                    instance_id,
                    pal,
                });
            }
        }

        containers.push(PalContainerView {
            kind: *kind,
            id: id_hex,
            slot_count,
            slots,
            missing: false,
        });
    }

    Ok(PlayerPalStorage {
        player_uid: uid,
        containers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pal_container_kind_strings_are_stable() {
        assert_eq!(PalContainerKind::Party.as_str(), "party");
        assert_eq!(PalContainerKind::Storage.as_str(), "storage");
        // The two tables must not collide at the wire level either — a UI keyed by
        // these strings would silently merge a party with a backpack.
        for (pal_kind, _) in PAL_CONTAINER_KINDS {
            for (item_kind, _) in CONTAINER_KINDS {
                assert_ne!(pal_kind.as_str(), item_kind.as_str());
            }
        }
    }

    #[test]
    fn container_kind_strings_are_stable() {
        assert_eq!(ContainerKind::Common.as_str(), "common");
        assert_eq!(ContainerKind::DropSlot.as_str(), "drop_slot");
        // Every kind in the table must have a distinct wire name.
        let mut names: Vec<&str> = CONTAINER_KINDS.iter().map(|(k, _)| k.as_str()).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate ContainerKind wire name");
    }

    #[test]
    fn a_non_player_save_is_rejected() {
        // Not a GVAS file at all; the point is that it fails rather than panicking.
        assert!(container_ids(b"not a save").is_err());
    }
}
