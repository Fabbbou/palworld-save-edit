//! Boundary coverage: open -> edit -> export across wasm-bindgen.
//!
//! The save is built synthetically rather than loaded from `fixtures/` — fixtures are
//! gitignored (they contain SteamIDs and player names) and a browser test can't read
//! the filesystem anyway. The *format* work is verified against real saves by
//! `palsave`'s own native fixture tests; what these tests exist to prove is that
//! handles, view models, and typed errors cross the boundary intact.

use palsave::container::{self, Algorithm, Container, Passes};
use palsave::gvas::header::{GVAS_MAGIC, Header};
use palsave::gvas::primitives::{FString, write_fstring, write_u32_le};
use palsave::gvas::property::{PropertyTag, TagExtra, none_terminator, write_property_tag};
use palsave::rawdata::group;
use palsave_wasm::open;
use wasm_bindgen_test::*;

// Runs under Node (`wasm-pack test --node`). These tests touch no DOM or Web API —
// only wasm-bindgen marshalling — so the browser runner would add a webdriver
// dependency without testing anything extra. Add
// `wasm_bindgen_test_configure!(run_in_browser)` here if a future test needs one.

const GUILD_ID: [u8; 16] = [
    0xab, 0xf7, 0x1e, 0xd8, 0x8a, 0xb8, 0x34, 0x4d, 0xa7, 0xe4, 0x62, 0x94, 0x85, 0x8d, 0x5d, 0x05,
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

/// A minimal but structurally real Level.sav: header, `worldSaveData`, one
/// `GroupSaveDataMap` entry. Built inner-to-outer so every `size` field is exact.
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
