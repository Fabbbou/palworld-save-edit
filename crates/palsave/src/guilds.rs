//! Task-level guild operations, composed from the container / gvas / rawdata / edit
//! layers below. This is the level `palsave-wasm` binds to: it exists so that the
//! wasm crate stays a pure binding shim with no logic of its own (see `CLAUDE.md`),
//! and so all of it stays natively testable with plain `cargo test`.
//!
//! Everything here takes and returns *decompressed GVAS* bytes. Container
//! compression is the caller's business — see `crate::container`.

use crate::edit::{self, EditError};
use crate::gvas::primitives::{FString, Guid};
use crate::gvas::value::{StructValue, Value, materialize_property};
use crate::gvas::{GvasError, GvasFile, PropertyEntry};
use crate::rawdata::error::RawDataError;
use crate::rawdata::group::{self, GroupData, GroupVariant};
use std::fmt;

const GROUP_MAP_PATH: &str = "worldSaveData.GroupSaveDataMap";
const GROUP_TYPE_PATH: &str = "worldSaveData.GroupSaveDataMap.Value.GroupType";
const RAW_DATA_PATH: &str = "worldSaveData.GroupSaveDataMap.Value.RawData";

#[derive(Debug)]
pub enum GuildError {
    Gvas(GvasError),
    RawData(RawDataError),
    Edit(EditError),
    /// This save has no `worldSaveData.GroupSaveDataMap` — e.g. it's a
    /// `Players/*.sav` or `LevelMeta.sav`, not a `Level.sav`.
    NoGroupMap,
    GuildNotFound {
        id: String,
    },
    /// The requested group exists but isn't a named guild (an `Organization` has no
    /// `guild_name` field at all).
    NotANamedGuild {
        id: String,
    },
    MalformedGuildId {
        id: String,
    },
}

impl GuildError {
    /// Stable, machine-readable discriminant for the wasm boundary — callers should
    /// branch on this, never on the display string.
    pub fn code(&self) -> &'static str {
        match self {
            GuildError::Gvas(_) => "gvas_parse_failed",
            GuildError::RawData(_) => "rawdata_decode_failed",
            GuildError::Edit(_) => "edit_failed",
            GuildError::NoGroupMap => "no_group_map",
            GuildError::GuildNotFound { .. } => "guild_not_found",
            GuildError::NotANamedGuild { .. } => "not_a_named_guild",
            GuildError::MalformedGuildId { .. } => "malformed_guild_id",
        }
    }
}

impl From<GvasError> for GuildError {
    fn from(e: GvasError) -> Self {
        GuildError::Gvas(e)
    }
}
impl From<RawDataError> for GuildError {
    fn from(e: RawDataError) -> Self {
        GuildError::RawData(e)
    }
}
impl From<EditError> for GuildError {
    fn from(e: EditError) -> Self {
        GuildError::Edit(e)
    }
}

impl fmt::Display for GuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GuildError::Gvas(e) => write!(f, "{e}"),
            GuildError::RawData(e) => write!(f, "{e}"),
            GuildError::Edit(e) => write!(f, "{e}"),
            GuildError::NoGroupMap => write!(f, "this save has no worldSaveData.GroupSaveDataMap"),
            GuildError::GuildNotFound { id } => write!(f, "no guild with id {id}"),
            GuildError::NotANamedGuild { id } => write!(f, "group {id} is not a named guild"),
            GuildError::MalformedGuildId { id } => {
                write!(f, "malformed guild id {id:?}: want 32 hex chars")
            }
        }
    }
}

impl std::error::Error for GuildError {}

/// A `Guid` rendered as 32 lowercase hex chars. Deliberately *not* dashed
/// UUID form: Unreal's `FGuid` field order vs. RFC-4122 byte order is a
/// well-known source of confusion, and this project never needs to interoperate
/// with an external UUID parser — it only needs a stable, reversible handle.
pub fn guid_to_hex(guid: &Guid) -> String {
    let mut s = String::with_capacity(32);
    for byte in guid {
        s.push_str(&format!("{byte:02x}"));
    }
    s
}

pub fn hex_to_guid(hex: &str) -> Option<Guid> {
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Small view model: what a guild list needs, and nothing proportional to save size.
#[derive(Debug, Clone, PartialEq)]
pub struct GuildSummary {
    pub id: String,
    pub group_type: String,
    pub name: String,
    pub member_count: usize,
    pub base_camp_level: i32,
    pub pal_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildMember {
    pub player_uid: String,
    pub player_name: String,
    pub last_online_real_time: i64,
    /// Per-member guild role, present only on the newer `PostUpdate` tail shape
    /// (see `rawdata::group::GuildTail`).
    pub role: Option<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildDetail {
    pub summary: GuildSummary,
    pub admin_player_uid: Option<String>,
    pub members: Vec<GuildMember>,
}

/// One group's location in the save: the `RawData` property to splice, plus the
/// already-decoded blob. Internal — callers get view models, not this.
struct GroupSlot {
    raw_entry: PropertyEntry,
    group_type: String,
    data: GroupData,
}

struct GroupWalk {
    world_entry: PropertyEntry,
    map_entry: PropertyEntry,
    groups: Vec<GroupSlot>,
}

/// Walks `worldSaveData.GroupSaveDataMap` once, decoding every group's RawData.
/// Shared by every public function here so the descent exists in exactly one place.
fn walk_groups(gvas: &[u8]) -> Result<GroupWalk, GuildError> {
    let file = GvasFile::parse(gvas)?;
    let engine_major = file.header.engine_version_major;
    let has_property_guid = file.header.has_property_guid();

    let world_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .ok_or(GuildError::NoGroupMap)?;
    let world_entry = file.properties[world_idx].clone();

    let Value::Struct(StructValue::Properties(world_children)) = file.materialize(world_idx)?
    else {
        return Err(GuildError::NoGroupMap);
    };
    let map_entry = world_children
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .ok_or(GuildError::NoGroupMap)?
        .clone();

    let map = materialize_property(
        gvas,
        &map_entry,
        engine_major,
        has_property_guid,
        GROUP_MAP_PATH,
    )?;
    let Value::Map(entries) = map else {
        return Err(GuildError::NoGroupMap);
    };

    let mut groups = Vec::with_capacity(entries.len());
    for (_key, value) in &entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            continue;
        };

        let Some(gt_entry) = fields.iter().find(|f| f.name == "GroupType") else {
            continue;
        };
        let Value::Enum(group_type) = materialize_property(
            gvas,
            gt_entry,
            engine_major,
            has_property_guid,
            GROUP_TYPE_PATH,
        )?
        else {
            continue;
        };
        let group_type = group_type.display_lossy();

        let Some(raw_entry) = fields.iter().find(|f| f.name == "RawData") else {
            continue;
        };
        let Value::Bytes(raw_bytes) = materialize_property(
            gvas,
            raw_entry,
            engine_major,
            has_property_guid,
            RAW_DATA_PATH,
        )?
        else {
            continue;
        };

        let data = group::decode(&raw_bytes, &group_type)?;
        groups.push(GroupSlot {
            raw_entry: raw_entry.clone(),
            group_type,
            data,
        });
    }

    Ok(GroupWalk {
        world_entry,
        map_entry,
        groups,
    })
}

fn summarize(slot: &GroupSlot) -> GuildSummary {
    let id = guid_to_hex(&slot.data.group_id);
    let pal_count = slot.data.individual_character_handle_ids.len();
    let (name, member_count, base_camp_level) = match &slot.data.data {
        GroupVariant::Guild { .. } | GroupVariant::IndependentGuild { .. } => {
            let (guild_name, base_camp_level) = named_guild_fields(&slot.data.data)
                .expect("Guild/IndependentGuild always carry a name and camp level");
            let members = match &slot.data.data {
                GroupVariant::Guild(g) => match &g.tail {
                    group::GuildTail::PreUpdate(t) => t.players.len(),
                    group::GuildTail::PostUpdate(t) => t.players.len(),
                },
                // An IndependentGuild is a single player's solo guild.
                _ => 1,
            };
            (guild_name.display_lossy(), members, base_camp_level)
        }
        GroupVariant::Organization(_) | GroupVariant::Unknown { .. } => {
            (slot.data.group_name.display_lossy(), 0, 0)
        }
    };

    GuildSummary {
        id,
        group_type: slot.group_type.clone(),
        name,
        member_count,
        base_camp_level,
        pal_count,
    }
}

fn named_guild_fields(variant: &GroupVariant) -> Option<(&FString, i32)> {
    match variant {
        GroupVariant::Guild(g) => Some((&g.guild_name, g.base_camp_level)),
        GroupVariant::IndependentGuild(g) => Some((&g.guild_name, g.base_camp_level)),
        _ => None,
    }
}

/// Every group in the save, as small view models. Size is proportional to the number
/// of guilds, never to the size of the save.
pub fn list(gvas: &[u8]) -> Result<Vec<GuildSummary>, GuildError> {
    Ok(walk_groups(gvas)?.groups.iter().map(summarize).collect())
}

/// One guild, with its member roster materialized on demand.
pub fn detail(gvas: &[u8], id: &str) -> Result<GuildDetail, GuildError> {
    let walk = walk_groups(gvas)?;
    let slot = walk
        .groups
        .iter()
        .find(|s| guid_to_hex(&s.data.group_id) == id)
        .ok_or_else(|| GuildError::GuildNotFound { id: id.to_string() })?;

    let summary = summarize(slot);
    let (admin_player_uid, members) = match &slot.data.data {
        GroupVariant::Guild(g) => {
            let admin = match &g.tail {
                group::GuildTail::PreUpdate(t) => guid_to_hex(&t.admin_player_uid),
                group::GuildTail::PostUpdate(t) => guid_to_hex(&t.admin_player_uid),
            };
            let members = match &g.tail {
                group::GuildTail::PreUpdate(t) => t
                    .players
                    .iter()
                    .map(|p| GuildMember {
                        player_uid: guid_to_hex(&p.player_uid),
                        player_name: p.player_name.display_lossy(),
                        last_online_real_time: p.last_online_real_time,
                        role: None,
                    })
                    .collect(),
                group::GuildTail::PostUpdate(t) => t
                    .players
                    .iter()
                    .map(|p| GuildMember {
                        player_uid: guid_to_hex(&p.player_uid),
                        player_name: p.player_name.display_lossy(),
                        last_online_real_time: p.last_online_real_time,
                        role: Some(p.role),
                    })
                    .collect(),
            };
            (Some(admin), members)
        }
        GroupVariant::IndependentGuild(g) => (
            Some(guid_to_hex(&g.player_uid)),
            vec![GuildMember {
                player_uid: guid_to_hex(&g.player_uid),
                player_name: g.player_name.display_lossy(),
                last_online_real_time: g.last_online_real_time,
                role: None,
            }],
        ),
        _ => (None, Vec::new()),
    };

    Ok(GuildDetail {
        summary,
        admin_player_uid,
        members,
    })
}

/// Renames one guild, returning a fresh GVAS buffer. Every byte outside the edited
/// guild's enclosing `GroupSaveDataMap` is copied verbatim — see `crate::edit` and
/// ADR-004.md. The result is structurally verified before it's returned; a buffer
/// that fails verification is an error, never a return value.
pub fn set_name(gvas: &[u8], id: &str, new_name: &str) -> Result<Vec<u8>, GuildError> {
    if hex_to_guid(id).is_none() {
        return Err(GuildError::MalformedGuildId { id: id.to_string() });
    }

    let file = GvasFile::parse(gvas)?;
    let has_property_guid = file.header.has_property_guid();
    let walk = walk_groups(gvas)?;

    let slot = walk
        .groups
        .iter()
        .find(|s| guid_to_hex(&s.data.group_id) == id)
        .ok_or_else(|| GuildError::GuildNotFound { id: id.to_string() })?;

    let mut data = slot.data.clone();
    let new_name_fstring = encode_name(new_name);
    match &mut data.data {
        GroupVariant::Guild(g) => g.guild_name = new_name_fstring,
        GroupVariant::IndependentGuild(g) => g.guild_name = new_name_fstring,
        _ => return Err(GuildError::NotANamedGuild { id: id.to_string() }),
    }

    let new_blob = group::encode(&data);
    let splices = edit::replace_property_value(
        gvas,
        &[&walk.world_entry, &walk.map_entry, &slot.raw_entry],
        edit::byte_array_value(&new_blob),
        has_property_guid,
    )?;
    let edited = splices.apply(gvas)?;
    edit::verify_reparses(&edited)?;
    Ok(edited)
}

/// ASCII names use the 1-byte-per-char form; anything else uses UTF-16LE. Both get
/// the null terminator the format expects, counted in the length field — matching
/// `FString`'s own round-trip rules in `gvas::primitives`.
fn encode_name(name: &str) -> FString {
    if name.is_empty() {
        FString::Empty
    } else if name.is_ascii() {
        FString::Ascii {
            content: name.as_bytes().to_vec(),
            trailing: vec![0],
        }
    } else {
        FString::Utf16 {
            content: name.encode_utf16().collect(),
            trailing: vec![0, 0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_hex_round_trips() {
        // Obviously-synthetic: test data, not a GUID lifted from a real save.
        let guid: Guid = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff,
        ];
        let hex = guid_to_hex(&guid);
        assert_eq!(hex, "00112233445566778899aabbccddeeff");
        assert_eq!(hex_to_guid(&hex), Some(guid));
    }

    #[test]
    fn malformed_hex_is_rejected() {
        assert_eq!(hex_to_guid("too short"), None);
        assert_eq!(hex_to_guid(&"z".repeat(32)), None);
    }

    #[test]
    fn encode_name_picks_the_right_fstring_form() {
        assert_eq!(encode_name(""), FString::Empty);
        assert_eq!(
            encode_name("Guild"),
            FString::Ascii {
                content: b"Guild".to_vec(),
                trailing: vec![0]
            }
        );
        let utf16 = encode_name("ギルド");
        assert!(matches!(utf16, FString::Utf16 { .. }));
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(GuildError::NoGroupMap.code(), "no_group_map");
        assert_eq!(
            GuildError::GuildNotFound { id: "x".into() }.code(),
            "guild_not_found"
        );
        assert_eq!(
            GuildError::MalformedGuildId { id: "x".into() }.code(),
            "malformed_guild_id"
        );
    }
}
