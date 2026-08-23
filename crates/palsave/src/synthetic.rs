//! Synthetic Palworld saves, built from this crate's own writers.
//!
//! Real saves are gitignored — they carry SteamIDs and player names — so anything
//! that needs a `.sav` in CI (the wasm boundary tests, the browser end-to-end suite)
//! has to construct one. These builders emit **structurally real** files: a proper
//! GVAS header, a `worldSaveData` holding a guild, a character-map entry and an item
//! container, plus a matching player save. Built inner-to-outer so every `size` field
//! is exact, which is what makes them useful as parser input rather than just bytes.
//!
//! Behind the `synthetic` feature so none of it ships in the released wasm. The
//! feature is enabled for `palsave-wasm`'s dev-dependency, so `cargo test` and
//! `clippy --all-targets` see it while `cargo build --release` does not.

use crate::container::{self, Algorithm, Container, Passes};
use crate::gvas::header::{GVAS_MAGIC, Header};
use crate::gvas::primitives::{FString, write_fstring, write_i32_le, write_u32_le};
use crate::gvas::property::{PropertyTag, TagExtra, none_terminator, write_property_tag};
use crate::rawdata::{group, item_container};

/// Obviously-synthetic: this is test data, not a GUID lifted from a real save.
pub const GUILD_ID: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

pub fn ascii(s: &str) -> FString {
    FString::Ascii {
        content: s.as_bytes().to_vec(),
        trailing: vec![0],
    }
}

/// One guild's RawData blob, in the current (`PostUpdate` tail) shape.
fn guild_blob() -> Vec<u8> {
    group::encode(&group::GroupData {
        group_id: GUILD_ID,
        group_name: ascii("test-group"),
        individual_character_handle_ids: vec![group::CharacterHandle {
            guid: [1u8; 16],
            instance_id: [2u8; 16],
        }],
        data: group::GroupVariant::Guild(group::GuildGroup {
            org_type: 0,
            leading_bytes: [0; 4],
            base_ids: vec![],
            unknown_1: 0,
            base_camp_level: 7,
            map_object_instance_ids_base_camp_points: vec![],
            guild_name: ascii("Original Name"),
            last_guild_name_modifier_player_uid: [0u8; 16],
            guild_markers: vec![],
            tail: group::GuildTail::PostUpdate(group::GuildTailPostUpdate {
                guild_chest_allowed_roles: vec![1, 2],
                unknown_i32: 0,
                admin_player_uid: [9u8; 16],
                players: vec![group::GuildPlayerWithRole {
                    player_uid: [9u8; 16],
                    last_online_real_time: 682105930000,
                    player_name: ascii("Tester"),
                    role: 1,
                }],
                role_permissions: vec![],
                trailing_bytes: [0; 4],
            }),
        }),
    })
}

/// The `GroupSaveDataMap` entry value: a property list carrying GroupType + RawData.
fn group_entry_value() -> Vec<u8> {
    let mut out = Vec::new();

    let group_type_value = {
        let mut v = Vec::new();
        write_fstring(&mut v, &ascii(group::GUILD));
        v
    };
    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("GroupType"),
            type_name: ascii("EnumProperty"),
            size: group_type_value.len() as u32,
            index: 0,
            extra: TagExtra::Enum {
                enum_type: ascii("EPalGroupType"),
            },
            guid: None,
        },
        true,
    );
    out.extend_from_slice(&group_type_value);

    let blob = guild_blob();
    let raw_value = {
        let mut v = Vec::new();
        write_u32_le(&mut v, blob.len() as u32);
        v.extend_from_slice(&blob);
        v
    };
    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("RawData"),
            type_name: ascii("ArrayProperty"),
            size: raw_value.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("ByteProperty"),
            },
            guid: None,
        },
        true,
    );
    out.extend_from_slice(&raw_value);

    write_fstring(&mut out, &none_terminator());
    out
}

pub const PLAYER_UID: [u8; 16] = [
    // Little-endian 1 in the final u32 group, so this renders as
    // "00000000000000000000000000000001" — Palworld's own player-file naming.
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0,
];
pub const PLAYER_INSTANCE_ID: [u8; 16] = [0x77; 16];
/// The container the synthetic player's InventoryInfo points at, and the key of the
/// one entry in the synthetic level's ItemContainerSaveData. The join under test.
pub const CONTAINER_ID: [u8; 16] = [0x42; 16];

/// A `SaveParameter` property list for a player: the `IsPlayer` flag that classifies
/// the entry, plus a couple of stats to prove values survive the crossing.
fn player_save_parameter() -> Vec<u8> {
    let mut out = Vec::new();

    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("Level"),
            type_name: ascii("ByteProperty"),
            size: 1,
            index: 0,
            extra: TagExtra::Byte {
                enum_type: ascii("None"),
            },
            guid: None,
        },
        true,
    );
    out.push(34);

    let nick = {
        let mut v = Vec::new();
        write_fstring(&mut v, &ascii("Tester"));
        v
    };
    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("NickName"),
            type_name: ascii("StrProperty"),
            size: nick.len() as u32,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        },
        true,
    );
    out.extend_from_slice(&nick);

    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("IsPlayer"),
            type_name: ascii("BoolProperty"),
            size: 0,
            index: 0,
            extra: TagExtra::Bool(true),
            guid: None,
        },
        true,
    );

    write_fstring(&mut out, &none_terminator());
    out
}

/// The `PalCharacterData` RawData blob: a nested property list holding
/// `SaveParameter`, then 4 unknown bytes, a group id, and 4 trailing bytes.
fn player_raw_data() -> Vec<u8> {
    let save_parameter = player_save_parameter();

    let mut object = Vec::new();
    write_property_tag(
        &mut object,
        &PropertyTag {
            name: ascii("SaveParameter"),
            type_name: ascii("StructProperty"),
            size: save_parameter.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalIndividualCharacterSaveParameter"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    object.extend_from_slice(&save_parameter);
    write_fstring(&mut object, &none_terminator());

    let mut blob = object;
    blob.extend_from_slice(&[0, 0, 0, 0]); // unknown_bytes
    blob.extend_from_slice(&GUILD_ID); // group_id
    blob.extend_from_slice(&[0, 0, 0, 0]); // trailing_bytes
    blob
}

/// One `CharacterSaveParameterMap` entry. The key carries the PlayerUId, which is
/// where `characters::list_players` reads the uid from — not the value.
fn character_map_value() -> Vec<u8> {
    let mut v = Vec::new();
    write_u32_le(&mut v, 0); // keys-to-remove count
    write_u32_le(&mut v, 1); // entry count

    // Key: a property list (PlayerUId + InstanceId), per gvas::hints.
    let mut key = Vec::new();
    for (name, guid) in [
        ("PlayerUId", PLAYER_UID),
        ("InstanceId", PLAYER_INSTANCE_ID),
    ] {
        write_property_tag(
            &mut key,
            &PropertyTag {
                name: ascii(name),
                type_name: ascii("StructProperty"),
                size: 16,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii("Guid"),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );
        key.extend_from_slice(&guid);
    }
    write_fstring(&mut key, &none_terminator());
    v.extend_from_slice(&key);

    // Value: a property list holding the RawData blob.
    let blob = player_raw_data();
    let raw_value = {
        let mut r = Vec::new();
        write_u32_le(&mut r, blob.len() as u32);
        r.extend_from_slice(&blob);
        r
    };
    let mut value = Vec::new();
    write_property_tag(
        &mut value,
        &PropertyTag {
            name: ascii("RawData"),
            type_name: ascii("ArrayProperty"),
            size: raw_value.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("ByteProperty"),
            },
            guid: None,
        },
        true,
    );
    value.extend_from_slice(&raw_value);
    write_fstring(&mut value, &none_terminator());
    v.extend_from_slice(&value);

    v
}

/// One `ItemContainerSaveData` entry holding a single occupied slot.
fn item_container_map_value() -> Vec<u8> {
    let mut v = Vec::new();
    write_u32_le(&mut v, 0); // keys-to-remove count
    write_u32_le(&mut v, 1); // entry count

    // Key: { ID: Guid }
    let mut key = Vec::new();
    write_property_tag(
        &mut key,
        &PropertyTag {
            name: ascii("ID"),
            type_name: ascii("StructProperty"),
            size: 16,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("Guid"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    key.extend_from_slice(&CONTAINER_ID);
    write_fstring(&mut key, &none_terminator());
    v.extend_from_slice(&key);

    // One slot body: a property list carrying the slot's RawData blob.
    let slot_blob = item_container::encode_slot(&item_container::ItemContainerSlot {
        slot_index: 0,
        count: 5,
        item: item_container::ItemId {
            static_id: ascii("Wood"),
            dynamic_id: item_container::DynamicId {
                created_world_id: [0u8; 16],
                local_id_in_created_world: [0u8; 16],
            },
        },
        trailing_bytes: Vec::new(),
    });
    let slot_raw_value = {
        let mut r = Vec::new();
        write_u32_le(&mut r, slot_blob.len() as u32);
        r.extend_from_slice(&slot_blob);
        r
    };
    let mut slot_body = Vec::new();
    write_property_tag(
        &mut slot_body,
        &PropertyTag {
            name: ascii("RawData"),
            type_name: ascii("ArrayProperty"),
            size: slot_raw_value.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("ByteProperty"),
            },
            guid: None,
        },
        true,
    );
    slot_body.extend_from_slice(&slot_raw_value);
    write_fstring(&mut slot_body, &none_terminator());

    // Slots: ArrayProperty<StructProperty>. The element tag is written once, before
    // the bodies — and is present even for a zero-length array (see ADR-003.md).
    let slots_value = {
        let mut a = Vec::new();
        write_u32_le(&mut a, 1); // element count
        write_property_tag(
            &mut a,
            &PropertyTag {
                name: ascii("Slots"),
                type_name: ascii("StructProperty"),
                size: slot_body.len() as u32,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii("PalItemSlotSaveData"),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );
        a.extend_from_slice(&slot_body);
        a
    };

    let mut value = Vec::new();
    write_property_tag(
        &mut value,
        &PropertyTag {
            name: ascii("Slots"),
            type_name: ascii("ArrayProperty"),
            size: slots_value.len() as u32,
            index: 0,
            extra: TagExtra::Array {
                inner_type: ascii("StructProperty"),
            },
            guid: None,
        },
        true,
    );
    value.extend_from_slice(&slots_value);

    write_property_tag(
        &mut value,
        &PropertyTag {
            name: ascii("SlotNum"),
            type_name: ascii("IntProperty"),
            size: 4,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        },
        true,
    );
    write_i32_le(&mut value, 42);
    write_fstring(&mut value, &none_terminator());
    v.extend_from_slice(&value);

    v
}

/// A `Players/<uid>.sav`: the save class that identifies it, plus the `SaveData`
/// holding the player's uid and the container ids that make an inventory resolvable.
pub fn synthetic_player_sav() -> Vec<u8> {
    // InventoryInfo.CommonContainerId = { ID: Guid }
    let mut container_id_value = Vec::new();
    write_property_tag(
        &mut container_id_value,
        &PropertyTag {
            name: ascii("ID"),
            type_name: ascii("StructProperty"),
            size: 16,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("Guid"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    container_id_value.extend_from_slice(&CONTAINER_ID);
    write_fstring(&mut container_id_value, &none_terminator());

    let mut inventory_info = Vec::new();
    write_property_tag(
        &mut inventory_info,
        &PropertyTag {
            name: ascii("CommonContainerId"),
            type_name: ascii("StructProperty"),
            size: container_id_value.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalContainerId"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    inventory_info.extend_from_slice(&container_id_value);
    write_fstring(&mut inventory_info, &none_terminator());

    let mut save_data = Vec::new();
    write_property_tag(
        &mut save_data,
        &PropertyTag {
            name: ascii("PlayerUId"),
            type_name: ascii("StructProperty"),
            size: 16,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("Guid"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    save_data.extend_from_slice(&PLAYER_UID);
    write_property_tag(
        &mut save_data,
        &PropertyTag {
            name: ascii("InventoryInfo"),
            type_name: ascii("StructProperty"),
            size: inventory_info.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalPlayerInventoryInfo"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    save_data.extend_from_slice(&inventory_info);
    write_fstring(&mut save_data, &none_terminator());

    let mut gvas = Vec::new();
    player_header().write(&mut gvas);
    write_fstring(&mut gvas, &ascii("/Script/Pal.PalWorldPlayerSaveGame"));
    write_property_tag(
        &mut gvas,
        &PropertyTag {
            name: ascii("SaveData"),
            type_name: ascii("StructProperty"),
            size: save_data.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalWorldPlayerSaveData"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    gvas.extend_from_slice(&save_data);
    write_fstring(&mut gvas, &none_terminator());

    let template = Container {
        algorithm: Algorithm::Zlib,
        passes: Passes::One,
        was_cnk_wrapped: false,
        gvas: Vec::new(),
    };
    container::encode(&gvas, &template)
}

fn player_header() -> Header {
    Header {
        magic: GVAS_MAGIC,
        save_game_version: 3,
        package_version_ue4: 522,
        package_version_ue5: Some(1009),
        engine_version_major: 5,
        engine_version_minor: 1,
        engine_version_patch: 1,
        engine_version_build: 0,
        engine_version_branch: FString::Empty,
        custom_version: Some((3, vec![])),
    }
}

/// A minimal but structurally real Level.sav: header, `worldSaveData`, one
/// `GroupSaveDataMap` entry and one `CharacterSaveParameterMap` entry. Built
/// inner-to-outer so every `size` field is exact.
pub fn synthetic_sav() -> Vec<u8> {
    let map_value = {
        let mut v = Vec::new();
        write_u32_le(&mut v, 0); // keys-to-remove count
        write_u32_le(&mut v, 1); // entry count
        v.extend_from_slice(&GUILD_ID); // key: bare Guid (per gvas::hints)
        v.extend_from_slice(&group_entry_value());
        v
    };

    let world_value = {
        let mut v = Vec::new();
        write_property_tag(
            &mut v,
            &PropertyTag {
                name: ascii("GroupSaveDataMap"),
                type_name: ascii("MapProperty"),
                size: map_value.len() as u32,
                index: 0,
                extra: TagExtra::Map {
                    key_type: ascii("StructProperty"),
                    value_type: ascii("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        v.extend_from_slice(&map_value);

        let character_value = character_map_value();
        write_property_tag(
            &mut v,
            &PropertyTag {
                name: ascii("CharacterSaveParameterMap"),
                type_name: ascii("MapProperty"),
                size: character_value.len() as u32,
                index: 0,
                extra: TagExtra::Map {
                    key_type: ascii("StructProperty"),
                    value_type: ascii("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        v.extend_from_slice(&character_value);

        let item_value = item_container_map_value();
        write_property_tag(
            &mut v,
            &PropertyTag {
                name: ascii("ItemContainerSaveData"),
                type_name: ascii("MapProperty"),
                size: item_value.len() as u32,
                index: 0,
                extra: TagExtra::Map {
                    key_type: ascii("StructProperty"),
                    value_type: ascii("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        v.extend_from_slice(&item_value);

        write_fstring(&mut v, &none_terminator());
        v
    };

    let mut gvas = Vec::new();
    Header {
        magic: GVAS_MAGIC,
        save_game_version: 3,
        package_version_ue4: 522,
        package_version_ue5: Some(1009),
        engine_version_major: 5,
        engine_version_minor: 1,
        engine_version_patch: 1,
        engine_version_build: 0,
        engine_version_branch: FString::Empty,
        custom_version: Some((3, vec![])),
    }
    .write(&mut gvas);
    write_fstring(&mut gvas, &ascii("/Script/Pal.PalWorldSaveGame"));

    write_property_tag(
        &mut gvas,
        &PropertyTag {
            name: ascii("worldSaveData"),
            type_name: ascii("StructProperty"),
            size: world_value.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalWorldSaveData"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    gvas.extend_from_slice(&world_value);
    write_fstring(&mut gvas, &none_terminator());

    let template = Container {
        algorithm: Algorithm::Zlib,
        passes: Passes::One,
        was_cnk_wrapped: false,
        gvas: Vec::new(),
    };
    container::encode(&gvas, &template)
}
