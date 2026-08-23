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
//! Read-only. Editing items is a separate change with its own gates.

use crate::gvas::nav::{self, Cursor};
use crate::gvas::primitives::Guid;
use crate::gvas::{GvasError, GvasFile};
use crate::rawdata::error::RawDataError;
use crate::rawdata::item_container;
use crate::world::{self, WorldError};
use std::collections::BTreeMap;
use std::fmt;

pub const ITEM_CONTAINER_MAP: &str = "ItemContainerSaveData";

/// The container kinds reachable from a player's `SaveData`, in the order a UI
/// should show them. The `&str` is the property name under `InventoryInfo`.
///
/// `PalStorageContainerId` and `OtomoCharacterContainerId` are deliberately absent:
/// they live at `SaveData` top level and point into
/// `CharacterContainerSaveData`, a different map holding Pals rather than items,
/// which this crate has no slot decoder for yet.
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

#[derive(Debug)]
pub enum InventoryError {
    World(WorldError),
    Gvas(GvasError),
    RawData(RawDataError),
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

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InventoryError::World(e) => write!(f, "{e}"),
            InventoryError::Gvas(e) => write!(f, "{e}"),
            InventoryError::RawData(e) => write!(f, "{e}"),
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

/// Joins a player's container ids against `Level.sav`'s item containers.
///
/// One pass over the container map builds a guid -> position index; only the handful
/// of matching entries are then decoded. Decoding all 1488 containers to find six
/// would be pure waste on an 8.5 MB save.
pub fn player_inventory(
    level_gvas: &[u8],
    player_gvas: &[u8],
) -> Result<PlayerInventory, InventoryError> {
    let uid = player_uid(player_gvas)?;
    let wanted = container_ids(player_gvas)?;

    let map = world::open_map(level_gvas, ITEM_CONTAINER_MAP)?;

    // guid -> index into map.entries
    let mut index: BTreeMap<Guid, usize> = BTreeMap::new();
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

#[cfg(test)]
mod tests {
    use super::*;

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
