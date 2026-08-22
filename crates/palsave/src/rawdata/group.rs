//! `.worldSaveData.GroupSaveDataMap` value's "RawData" blob: guilds and
//! organizations. This is entirely game-specific and invisible to a generic UE GVAS
//! parser — cheahjs/palworld-save-tools (MIT), `rawdata/group.py`, was the only
//! written spec, but it's unchanged since 2024-02-03 (checked via the GitHub commits
//! API — no newer fix exists there) and it round-trips against synthetic data but not
//! real fixtures from a current Palworld build (see ADR-002.md for the byte-level
//! divergence that surfaced).
//!
//! The layout below is instead ported from `oMaN-Rod/uesave-rs` (branch
//! `pluggable-game-support`, MIT), `uesave/src/games/palworld/groups.rs` — an
//! actively maintained fork (pushed within the last day, as of this port) with
//! native, typed Palworld `RawData` support built into its archive reader/writer.
//! Field-by-field verified against real data: decoding the real "Guild" group in
//! `fixtures/Level.sav` with this layout reaches a clean, exact EOF and recovers
//! sane content throughout: `base_camp_level=13`, a guild name that reads back as
//! real text, and a named member. See `tests/fixtures.rs` for the fixture-backed test.
//!
//! `group_type` (Guild / IndependentGuild / Organization / anything else) isn't part
//! of this blob — it's the sibling `GroupType` EnumProperty's materialized value,
//! passed in by the caller. It selects which variant follows the common header
//! (group_id, group_name, individual_character_handle_ids). Everything else decodes
//! into `GroupVariant::Unknown`, keeping its bytes opaque rather than guessing.

use super::error::RawDataError;
use crate::gvas::primitives::{
    FString, Guid, read_fstring, read_guid, read_i32_le, read_i64_le, read_u8, read_u32_le,
    write_fstring, write_guid, write_i32_le, write_i64_le, write_u8, write_u32_le,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterHandle {
    pub guid: Guid,
    pub instance_id: Guid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildPlayer {
    pub player_uid: Guid,
    pub last_online_real_time: i64,
    pub player_name: FString,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildPlayerWithRole {
    pub player_uid: Guid,
    pub last_online_real_time: i64,
    pub player_name: FString,
    pub role: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildMarker {
    pub marker_id: Guid,
    pub icon_location: (f64, f64, f64),
    pub icon_type: i32,
    pub owner_player_uid: Guid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildRolePermission {
    pub role: u8,
    pub permissions: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildTailPreUpdate {
    pub admin_player_uid: Guid,
    pub players: Vec<GuildPlayer>,
    pub trailing_bytes: [u8; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildTailPostUpdate {
    pub guild_chest_allowed_roles: Vec<u8>,
    pub unknown_i32: i32,
    pub admin_player_uid: Guid,
    pub players: Vec<GuildPlayerWithRole>,
    pub role_permissions: Vec<GuildRolePermission>,
    pub trailing_bytes: [u8; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuildTail {
    PostUpdate(GuildTailPostUpdate),
    PreUpdate(GuildTailPreUpdate),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuildGroup {
    pub org_type: u8,
    pub leading_bytes: [u8; 4],
    pub base_ids: Vec<Guid>,
    pub unknown_1: i32,
    pub base_camp_level: i32,
    pub map_object_instance_ids_base_camp_points: Vec<Guid>,
    pub guild_name: FString,
    pub last_guild_name_modifier_player_uid: Guid,
    pub guild_markers: Vec<GuildMarker>,
    pub tail: GuildTail,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndependentGuildGroup {
    pub org_type: u8,
    pub base_camp_level: i32,
    pub map_object_instance_ids_base_camp_points: Vec<Guid>,
    pub guild_name: FString,
    pub player_uid: Guid,
    pub guild_name_2: FString,
    pub last_online_real_time: i64,
    pub player_name: FString,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrganizationGroup {
    pub org_type: u8,
    /// Unexplained — likely more structured than "12 opaque bytes" (one of them
    /// varies 2..8 across the 7 real Organization groups checked, looking like a
    /// small sequential id), but the upstream reference we're porting keeps it
    /// opaque too, and round-trip correctness doesn't require decoding it further.
    pub trailing_bytes: [u8; 12],
}

#[derive(Debug, Clone, PartialEq)]
pub enum GroupVariant {
    Guild(GuildGroup),
    IndependentGuild(IndependentGuildGroup),
    Organization(OrganizationGroup),
    Unknown { remaining_data: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupData {
    pub group_id: Guid,
    pub group_name: FString,
    pub individual_character_handle_ids: Vec<CharacterHandle>,
    pub data: GroupVariant,
}

pub const GUILD: &str = "EPalGroupType::Guild";
pub const INDEPENDENT_GUILD: &str = "EPalGroupType::IndependentGuild";
pub const ORGANIZATION: &str = "EPalGroupType::Organization";

fn read_guid_array(buf: &[u8], pos: &mut usize) -> Result<Vec<Guid>, RawDataError> {
    let count = read_u32_le(buf, pos)?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(read_guid(buf, pos)?);
    }
    Ok(out)
}

fn write_guid_array(out: &mut Vec<u8>, items: &[Guid]) {
    write_u32_le(out, items.len() as u32);
    for g in items {
        write_guid(out, g);
    }
}

fn read_player(buf: &[u8], pos: &mut usize) -> Result<GuildPlayer, RawDataError> {
    Ok(GuildPlayer {
        player_uid: read_guid(buf, pos)?,
        last_online_real_time: read_i64_le(buf, pos)?,
        player_name: read_fstring(buf, pos)?,
    })
}

fn write_player(out: &mut Vec<u8>, p: &GuildPlayer) {
    write_guid(out, &p.player_uid);
    write_i64_le(out, p.last_online_real_time);
    write_fstring(out, &p.player_name);
}

fn read_player_with_role(buf: &[u8], pos: &mut usize) -> Result<GuildPlayerWithRole, RawDataError> {
    let player = read_player(buf, pos)?;
    Ok(GuildPlayerWithRole {
        player_uid: player.player_uid,
        last_online_real_time: player.last_online_real_time,
        player_name: player.player_name,
        role: read_u8(buf, pos)?,
    })
}

fn write_player_with_role(out: &mut Vec<u8>, p: &GuildPlayerWithRole) {
    write_guid(out, &p.player_uid);
    write_i64_le(out, p.last_online_real_time);
    write_fstring(out, &p.player_name);
    write_u8(out, p.role);
}

fn read_bytes_fixed<const N: usize>(buf: &[u8], pos: &mut usize) -> Result<[u8; N], RawDataError> {
    let mut out = [0u8; N];
    for slot in out.iter_mut() {
        *slot = read_u8(buf, pos)?;
    }
    Ok(out)
}

impl GuildTailPreUpdate {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        let admin_player_uid = read_guid(buf, pos)?;
        let player_count = read_u32_le(buf, pos)?;
        let mut players = Vec::with_capacity(player_count as usize);
        for _ in 0..player_count {
            players.push(read_player(buf, pos)?);
        }
        let trailing_bytes = read_bytes_fixed(buf, pos)?;
        Ok(GuildTailPreUpdate {
            admin_player_uid,
            players,
            trailing_bytes,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_guid(out, &self.admin_player_uid);
        write_u32_le(out, self.players.len() as u32);
        for p in &self.players {
            write_player(out, p);
        }
        out.extend_from_slice(&self.trailing_bytes);
    }
}

impl GuildTailPostUpdate {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        let chest_role_count = read_u32_le(buf, pos)?;
        let mut guild_chest_allowed_roles = Vec::with_capacity(chest_role_count as usize);
        for _ in 0..chest_role_count {
            guild_chest_allowed_roles.push(read_u8(buf, pos)?);
        }
        let unknown_i32 = read_i32_le(buf, pos)?;
        let admin_player_uid = read_guid(buf, pos)?;
        let player_count = read_u32_le(buf, pos)?;
        let mut players = Vec::with_capacity(player_count as usize);
        for _ in 0..player_count {
            players.push(read_player_with_role(buf, pos)?);
        }
        let permission_count = read_u32_le(buf, pos)?;
        let mut role_permissions = Vec::with_capacity(permission_count as usize);
        for _ in 0..permission_count {
            let role = read_u8(buf, pos)?;
            let perm_count = read_u32_le(buf, pos)?;
            let mut permissions = Vec::with_capacity(perm_count as usize);
            for _ in 0..perm_count {
                permissions.push(read_u8(buf, pos)?);
            }
            role_permissions.push(GuildRolePermission { role, permissions });
        }
        let trailing_bytes = read_bytes_fixed(buf, pos)?;
        Ok(GuildTailPostUpdate {
            guild_chest_allowed_roles,
            unknown_i32,
            admin_player_uid,
            players,
            role_permissions,
            trailing_bytes,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_u32_le(out, self.guild_chest_allowed_roles.len() as u32);
        out.extend_from_slice(&self.guild_chest_allowed_roles);
        write_i32_le(out, self.unknown_i32);
        write_guid(out, &self.admin_player_uid);
        write_u32_le(out, self.players.len() as u32);
        for p in &self.players {
            write_player_with_role(out, p);
        }
        write_u32_le(out, self.role_permissions.len() as u32);
        for rp in &self.role_permissions {
            write_u8(out, rp.role);
            write_u32_le(out, rp.permissions.len() as u32);
            out.extend_from_slice(&rp.permissions);
        }
        out.extend_from_slice(&self.trailing_bytes);
    }
}

impl GuildTail {
    /// Tries the newer `PostUpdate` shape first; accepts it only if it decodes
    /// cleanly *and* consumes every remaining byte. Otherwise rewinds and reads the
    /// older `PreUpdate` shape instead. Ported from `PalGuildTail::read`, which does
    /// the same probe-and-rewind on a seekable stream.
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        let start = *pos;
        let mut probe = start;
        if let Ok(tail) = GuildTailPostUpdate::read(buf, &mut probe)
            && probe == buf.len()
        {
            *pos = probe;
            return Ok(GuildTail::PostUpdate(tail));
        }
        *pos = start;
        Ok(GuildTail::PreUpdate(GuildTailPreUpdate::read(buf, pos)?))
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            GuildTail::PostUpdate(t) => t.write(out),
            GuildTail::PreUpdate(t) => t.write(out),
        }
    }
}

impl GuildMarker {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        use crate::gvas::primitives::read_f64_le;
        Ok(GuildMarker {
            marker_id: read_guid(buf, pos)?,
            icon_location: (
                read_f64_le(buf, pos)?,
                read_f64_le(buf, pos)?,
                read_f64_le(buf, pos)?,
            ),
            icon_type: read_i32_le(buf, pos)?,
            owner_player_uid: read_guid(buf, pos)?,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_guid(out, &self.marker_id);
        out.extend_from_slice(&self.icon_location.0.to_bits().to_le_bytes());
        out.extend_from_slice(&self.icon_location.1.to_bits().to_le_bytes());
        out.extend_from_slice(&self.icon_location.2.to_bits().to_le_bytes());
        write_i32_le(out, self.icon_type);
        write_guid(out, &self.owner_player_uid);
    }
}

impl GuildGroup {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        let org_type = read_u8(buf, pos)?;
        let leading_bytes = read_bytes_fixed(buf, pos)?;
        let base_ids = read_guid_array(buf, pos)?;
        let unknown_1 = read_i32_le(buf, pos)?;
        let base_camp_level = read_i32_le(buf, pos)?;
        let map_object_instance_ids_base_camp_points = read_guid_array(buf, pos)?;
        let guild_name = read_fstring(buf, pos)?;
        let last_guild_name_modifier_player_uid = read_guid(buf, pos)?;
        let marker_count = read_u32_le(buf, pos)?;
        let mut guild_markers = Vec::with_capacity(marker_count as usize);
        for _ in 0..marker_count {
            guild_markers.push(GuildMarker::read(buf, pos)?);
        }
        let tail = GuildTail::read(buf, pos)?;
        Ok(GuildGroup {
            org_type,
            leading_bytes,
            base_ids,
            unknown_1,
            base_camp_level,
            map_object_instance_ids_base_camp_points,
            guild_name,
            last_guild_name_modifier_player_uid,
            guild_markers,
            tail,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_u8(out, self.org_type);
        out.extend_from_slice(&self.leading_bytes);
        write_guid_array(out, &self.base_ids);
        write_i32_le(out, self.unknown_1);
        write_i32_le(out, self.base_camp_level);
        write_guid_array(out, &self.map_object_instance_ids_base_camp_points);
        write_fstring(out, &self.guild_name);
        write_guid(out, &self.last_guild_name_modifier_player_uid);
        write_u32_le(out, self.guild_markers.len() as u32);
        for m in &self.guild_markers {
            m.write(out);
        }
        self.tail.write(out);
    }
}

impl IndependentGuildGroup {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        let org_type = read_u8(buf, pos)?;
        let base_camp_level = read_i32_le(buf, pos)?;
        let map_object_instance_ids_base_camp_points = read_guid_array(buf, pos)?;
        let guild_name = read_fstring(buf, pos)?;
        let player_uid = read_guid(buf, pos)?;
        let guild_name_2 = read_fstring(buf, pos)?;
        let last_online_real_time = read_i64_le(buf, pos)?;
        let player_name = read_fstring(buf, pos)?;
        Ok(IndependentGuildGroup {
            org_type,
            base_camp_level,
            map_object_instance_ids_base_camp_points,
            guild_name,
            player_uid,
            guild_name_2,
            last_online_real_time,
            player_name,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_u8(out, self.org_type);
        write_i32_le(out, self.base_camp_level);
        write_guid_array(out, &self.map_object_instance_ids_base_camp_points);
        write_fstring(out, &self.guild_name);
        write_guid(out, &self.player_uid);
        write_fstring(out, &self.guild_name_2);
        write_i64_le(out, self.last_online_real_time);
        write_fstring(out, &self.player_name);
    }
}

impl OrganizationGroup {
    fn read(buf: &[u8], pos: &mut usize) -> Result<Self, RawDataError> {
        Ok(OrganizationGroup {
            org_type: read_u8(buf, pos)?,
            trailing_bytes: read_bytes_fixed(buf, pos)?,
        })
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_u8(out, self.org_type);
        out.extend_from_slice(&self.trailing_bytes);
    }
}

pub fn decode(bytes: &[u8], group_type: &str) -> Result<GroupData, RawDataError> {
    let mut pos = 0usize;

    let group_id = read_guid(bytes, &mut pos)?;
    let group_name = read_fstring(bytes, &mut pos)?;
    let handle_count = read_u32_le(bytes, &mut pos)?;
    let mut individual_character_handle_ids = Vec::with_capacity(handle_count as usize);
    for _ in 0..handle_count {
        individual_character_handle_ids.push(CharacterHandle {
            guid: read_guid(bytes, &mut pos)?,
            instance_id: read_guid(bytes, &mut pos)?,
        });
    }

    let data = match group_type {
        GUILD => GroupVariant::Guild(GuildGroup::read(bytes, &mut pos)?),
        INDEPENDENT_GUILD => {
            GroupVariant::IndependentGuild(IndependentGuildGroup::read(bytes, &mut pos)?)
        }
        ORGANIZATION => GroupVariant::Organization(OrganizationGroup::read(bytes, &mut pos)?),
        _ => {
            let remaining_data = bytes[pos..].to_vec();
            pos = bytes.len();
            GroupVariant::Unknown { remaining_data }
        }
    };

    if pos != bytes.len() {
        return Err(RawDataError::NotExhausted {
            consumed: pos,
            total: bytes.len(),
        });
    }

    Ok(GroupData {
        group_id,
        group_name,
        individual_character_handle_ids,
        data,
    })
}

pub fn encode(data: &GroupData) -> Vec<u8> {
    let mut out = Vec::new();
    write_guid(&mut out, &data.group_id);
    write_fstring(&mut out, &data.group_name);
    write_u32_le(&mut out, data.individual_character_handle_ids.len() as u32);
    for h in &data.individual_character_handle_ids {
        write_guid(&mut out, &h.guid);
        write_guid(&mut out, &h.instance_id);
    }

    match &data.data {
        GroupVariant::Guild(g) => g.write(&mut out),
        GroupVariant::IndependentGuild(g) => g.write(&mut out),
        GroupVariant::Organization(g) => g.write(&mut out),
        GroupVariant::Unknown { remaining_data } => out.extend_from_slice(remaining_data),
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii(s: &str) -> FString {
        FString::Ascii {
            content: s.as_bytes().to_vec(),
            trailing: vec![0],
        }
    }

    fn base_data(data: GroupVariant) -> GroupData {
        GroupData {
            group_id: [1u8; 16],
            group_name: ascii("Group"),
            individual_character_handle_ids: vec![CharacterHandle {
                guid: [2u8; 16],
                instance_id: [3u8; 16],
            }],
            data,
        }
    }

    #[test]
    fn organization_round_trips() {
        let data = base_data(GroupVariant::Organization(OrganizationGroup {
            org_type: 5,
            trailing_bytes: [7u8; 12],
        }));
        let bytes = encode(&data);
        assert_eq!(decode(&bytes, ORGANIZATION).unwrap(), data);
    }

    #[test]
    fn independent_guild_round_trips() {
        let data = base_data(GroupVariant::IndependentGuild(IndependentGuildGroup {
            org_type: 1,
            base_camp_level: 4,
            map_object_instance_ids_base_camp_points: vec![[9u8; 16]],
            guild_name: ascii("Solo"),
            player_uid: [8u8; 16],
            guild_name_2: ascii("Solo"),
            last_online_real_time: 999,
            player_name: ascii("Bob"),
        }));
        let bytes = encode(&data);
        assert_eq!(decode(&bytes, INDEPENDENT_GUILD).unwrap(), data);
    }

    #[test]
    fn guild_pre_update_round_trips() {
        let data = base_data(GroupVariant::Guild(GuildGroup {
            org_type: 0,
            leading_bytes: [0; 4],
            base_ids: vec![],
            unknown_1: 0,
            base_camp_level: 3,
            map_object_instance_ids_base_camp_points: vec![],
            guild_name: ascii("MyGuild"),
            last_guild_name_modifier_player_uid: [0u8; 16],
            guild_markers: vec![],
            tail: GuildTail::PreUpdate(GuildTailPreUpdate {
                admin_player_uid: [5u8; 16],
                players: vec![GuildPlayer {
                    player_uid: [6u8; 16],
                    last_online_real_time: 12345,
                    player_name: ascii("Alice"),
                }],
                trailing_bytes: [0; 4],
            }),
        }));
        let bytes = encode(&data);
        let decoded = decode(&bytes, GUILD).unwrap();
        assert_eq!(decoded, data);
        assert!(matches!(
            decoded.data,
            GroupVariant::Guild(GuildGroup {
                tail: GuildTail::PreUpdate(_),
                ..
            })
        ));
    }

    #[test]
    fn guild_post_update_round_trips() {
        let data = base_data(GroupVariant::Guild(GuildGroup {
            org_type: 0,
            leading_bytes: [1, 2, 3, 4],
            base_ids: vec![[10u8; 16]],
            unknown_1: 7,
            base_camp_level: 13,
            map_object_instance_ids_base_camp_points: vec![[11u8; 16], [12u8; 16]],
            guild_name: ascii("Example Guild"),
            last_guild_name_modifier_player_uid: [0u8; 16],
            guild_markers: vec![GuildMarker {
                marker_id: [13u8; 16],
                icon_location: (1.5, 2.5, 3.5),
                icon_type: 2,
                owner_player_uid: [14u8; 16],
            }],
            tail: GuildTail::PostUpdate(GuildTailPostUpdate {
                guild_chest_allowed_roles: vec![1, 2],
                unknown_i32: 0,
                admin_player_uid: [6u8; 16],
                players: vec![GuildPlayerWithRole {
                    player_uid: [6u8; 16],
                    last_online_real_time: 682105930000,
                    player_name: ascii("Member One"),
                    role: 1,
                }],
                role_permissions: vec![GuildRolePermission {
                    role: 1,
                    permissions: vec![1, 2, 3],
                }],
                trailing_bytes: [0; 4],
            }),
        }));
        let bytes = encode(&data);
        let decoded = decode(&bytes, GUILD).unwrap();
        assert_eq!(decoded, data);
        assert!(matches!(
            decoded.data,
            GroupVariant::Guild(GuildGroup {
                tail: GuildTail::PostUpdate(_),
                ..
            })
        ));
    }

    #[test]
    fn unknown_group_type_keeps_bytes_opaque() {
        let data = base_data(GroupVariant::Unknown {
            remaining_data: vec![1, 2, 3, 4, 5],
        });
        let bytes = encode(&data);
        assert_eq!(
            decode(&bytes, "EPalGroupType::SomethingFuture").unwrap(),
            data
        );
    }

    #[test]
    fn trailing_bytes_are_rejected_not_dropped() {
        let data = base_data(GroupVariant::Organization(OrganizationGroup {
            org_type: 0,
            trailing_bytes: [0u8; 12],
        }));
        let mut bytes = encode(&data);
        bytes.push(0xFF);
        assert!(matches!(
            decode(&bytes, ORGANIZATION),
            Err(RawDataError::NotExhausted { .. })
        ));
    }
}
