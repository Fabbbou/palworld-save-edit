//! Boundary coverage: open -> edit -> export across wasm-bindgen.
//!
//! The save is built synthetically rather than loaded from `fixtures/` — fixtures are
//! gitignored (they contain SteamIDs and player names) and a browser test can't read
//! the filesystem anyway. The *format* work is verified against real saves by
//! `palsave`'s own native fixture tests; what these tests exist to prove is that
//! handles, view models, and typed errors cross the boundary intact.

use palsave::container::{self, Algorithm, Container, Passes};
use palsave::gvas::header::{GVAS_MAGIC, Header};
use palsave::gvas::primitives::{FString, write_fstring, write_i32_le, write_u32_le};
use palsave::gvas::property::{PropertyTag, TagExtra, none_terminator, write_property_tag};
use palsave::rawdata::{group, item_container};
use palsave_wasm::open;
use wasm_bindgen_test::*;

// Runs under Node (`wasm-pack test --node`). These tests touch no DOM or Web API —
// only wasm-bindgen marshalling — so the browser runner would add a webdriver
// dependency without testing anything extra. Add
// `wasm_bindgen_test_configure!(run_in_browser)` here if a future test needs one.

/// Obviously-synthetic: this is test data, not a GUID lifted from a real save.
const GUILD_ID: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

fn ascii(s: &str) -> FString {
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

const PLAYER_UID: [u8; 16] = [
    // Little-endian 1 in the final u32 group, so this renders as
    // "00000000000000000000000000000001" — Palworld's own player-file naming.
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0,
];
const PLAYER_INSTANCE_ID: [u8; 16] = [0x77; 16];
/// The container the synthetic player's InventoryInfo points at, and the key of the
/// one entry in the synthetic level's ItemContainerSaveData. The join under test.
const CONTAINER_ID: [u8; 16] = [0x42; 16];

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
fn synthetic_player_sav() -> Vec<u8> {
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
fn synthetic_sav() -> Vec<u8> {
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

fn get(value: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(value, &JsValue::from_str(key)).unwrap()
}

use wasm_bindgen::JsValue;

#[wasm_bindgen_test]
fn open_then_summary_crosses_the_boundary() {
    let handle = open(&synthetic_sav()).expect("open");
    let summary = handle.summary().expect("summary");

    assert_eq!(
        get(&summary, "save_game_type").as_string().unwrap(),
        "/Script/Pal.PalWorldSaveGame"
    );
    assert_eq!(
        get(&summary, "engine_version").as_string().unwrap(),
        "5.1.1"
    );
    assert_eq!(
        get(&summary, "top_level_property_count").as_f64().unwrap(),
        1.0
    );

    let container = get(&summary, "container");
    assert_eq!(get(&container, "format").as_string().unwrap(), "PlZ");
    // A PlZ save round-trips as PlZ, so no downgrade warning is due.
    assert!(!get(&container, "will_downgrade_to_zlib").as_bool().unwrap());
}

#[wasm_bindgen_test]
fn list_guilds_returns_a_small_view_model() {
    let handle = open(&synthetic_sav()).expect("open");
    let list = handle.list_guilds().expect("listGuilds");
    let arr = js_sys::Array::from(&list);
    assert_eq!(arr.length(), 1);

    let first = arr.get(0);
    assert_eq!(get(&first, "name").as_string().unwrap(), "Original Name");
    assert_eq!(get(&first, "group_type").as_string().unwrap(), group::GUILD);
    assert_eq!(get(&first, "member_count").as_f64().unwrap(), 1.0);
    assert_eq!(get(&first, "base_camp_level").as_f64().unwrap(), 7.0);
}

#[wasm_bindgen_test]
fn guild_detail_keeps_large_tick_counts_exact() {
    let handle = open(&synthetic_sav()).expect("open");
    let list = js_sys::Array::from(&handle.list_guilds().unwrap());
    let id = get(&list.get(0), "id").as_string().unwrap();

    let detail = handle.guild(&id).expect("guild");
    let members = js_sys::Array::from(&get(&detail, "members"));
    assert_eq!(members.length(), 1);

    let member = members.get(0);
    assert_eq!(get(&member, "player_name").as_string().unwrap(), "Tester");
    // Crosses as a string precisely so it can't be rounded by JS number semantics.
    assert_eq!(
        get(&member, "last_online_real_time").as_string().unwrap(),
        "682105930000"
    );
    assert_eq!(get(&member, "role").as_f64().unwrap(), 1.0);
}

/// The phase gate: open -> edit -> export, all across the boundary, and the exported
/// bytes reopen with the edit intact.
#[wasm_bindgen_test]
fn open_edit_export_round_trips() {
    let mut handle = open(&synthetic_sav()).expect("open");
    let list = js_sys::Array::from(&handle.list_guilds().unwrap());
    let id = get(&list.get(0), "id").as_string().unwrap();

    const NEW_NAME: &str = "A Considerably Longer Guild Name";
    handle.set_guild_name(&id, NEW_NAME).expect("setGuildName");

    let exported = handle.export().expect("export");
    assert!(!exported.is_empty());

    // Reopen the exported container from scratch — the edit must survive
    // re-compression and re-parsing, not just live in the handle.
    let reopened = open(&exported).expect("reopen exported save");
    let after = js_sys::Array::from(&reopened.list_guilds().unwrap());
    assert_eq!(after.length(), 1);
    assert_eq!(get(&after.get(0), "name").as_string().unwrap(), NEW_NAME);
    assert_eq!(get(&after.get(0), "id").as_string().unwrap(), id);
    // The edit changed the name and nothing else about the guild.
    assert_eq!(get(&after.get(0), "member_count").as_f64().unwrap(), 1.0);
    assert_eq!(get(&after.get(0), "base_camp_level").as_f64().unwrap(), 7.0);
}

#[wasm_bindgen_test]
fn players_and_pals_cross_the_boundary() {
    let handle = open(&synthetic_sav()).expect("open");
    let players = js_sys::Array::from(&handle.list_players().expect("listPlayers"));
    assert_eq!(players.length(), 1);

    let player = players.get(0);
    assert_eq!(get(&player, "nickname").as_string().unwrap(), "Tester");
    assert_eq!(get(&player, "level").as_f64().unwrap(), 34.0);
    // Rendered in Unreal's convention, which is also the player's save filename.
    assert_eq!(
        get(&player, "uid").as_string().unwrap(),
        "00000000000000000000000000000001"
    );

    // player() agrees with the list.
    let uid = get(&player, "uid").as_string().unwrap();
    let detail = handle.player(&uid).expect("player");
    let summary = get(&detail, "summary");
    assert_eq!(get(&summary, "nickname").as_string().unwrap(), "Tester");
    // This synthetic save has no Pals, so the roster is empty rather than absent.
    assert_eq!(js_sys::Array::from(&get(&detail, "pals")).length(), 0);
    assert_eq!(
        js_sys::Array::from(&handle.pals_of(&uid).expect("palsOf")).length(),
        0
    );
}

/// The two-file join across the boundary: attach a player save, then resolve their
/// inventory out of the level.
#[wasm_bindgen_test]
fn attaching_a_player_save_resolves_their_inventory() {
    let mut handle = open(&synthetic_sav()).expect("open level");

    // The uid comes from the file, not from the caller.
    let uid = handle
        .attach_player_save(&synthetic_player_sav())
        .expect("attachPlayerSave");
    assert_eq!(uid, "00000000000000000000000000000001");

    let attached = js_sys::Array::from(&handle.attached_players().unwrap());
    assert_eq!(attached.length(), 1);

    let inv = handle.player_inventory(&uid).expect("playerInventory");
    assert_eq!(get(&inv, "player_uid").as_string().unwrap(), uid);

    let containers = js_sys::Array::from(&get(&inv, "containers"));
    assert_eq!(
        containers.length(),
        1,
        "only CommonContainerId is populated"
    );

    let common = containers.get(0);
    assert_eq!(get(&common, "kind").as_string().unwrap(), "common");
    assert_eq!(get(&common, "slot_count").as_f64().unwrap(), 42.0);
    assert!(!get(&common, "missing").as_bool().unwrap());

    let slots = js_sys::Array::from(&get(&common, "slots"));
    assert_eq!(slots.length(), 1);
    let slot = slots.get(0);
    assert_eq!(get(&slot, "static_id").as_string().unwrap(), "Wood");
    assert_eq!(get(&slot, "count").as_f64().unwrap(), 5.0);

    // Detaching really removes it.
    handle.detach_player_save(&uid);
    assert_eq!(
        js_sys::Array::from(&handle.attached_players().unwrap()).length(),
        0
    );
}

#[wasm_bindgen_test]
fn errors_cross_with_machine_readable_codes() {
    let Err(err) = open(b"not a save file at all") else {
        panic!("garbage bytes must not open")
    };
    assert_eq!(
        get(&err, "code").as_string().unwrap(),
        "container_decode_failed"
    );
    assert!(!get(&err, "message").as_string().unwrap().is_empty());

    let mut handle = open(&synthetic_sav()).expect("open");

    let err = handle.guild("not-a-guid").unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "guild_not_found");

    let err = handle.set_guild_name("nope", "x").unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "malformed_guild_id");

    let err = handle.player("nope").unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "malformed_uid");

    let err = handle.player(&"0".repeat(32)).unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "player_not_found");

    // A level save is not a player save, and the guard must say so rather than
    // silently attaching it and reporting an empty inventory.
    let err = handle.attach_player_save(&synthetic_sav()).unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "not_a_player_save");

    let err = handle.player_inventory(&"0".repeat(32)).unwrap_err();
    assert_eq!(
        get(&err, "code").as_string().unwrap(),
        "player_save_not_attached"
    );
}

#[wasm_bindgen_test]
fn diagnostics_report_clean_on_a_well_formed_save() {
    let handle = open(&synthetic_sav()).expect("open");
    let diag = handle.diagnostics().expect("diagnostics");

    assert_eq!(get(&diag, "engine_version").as_string().unwrap(), "5.1.1");
    assert_eq!(get(&diag, "container_format").as_string().unwrap(), "PlZ");
    let warnings = js_sys::Array::from(&get(&diag, "warnings"));
    assert_eq!(
        warnings.length(),
        0,
        "a well-formed save should produce no warnings"
    );
}
