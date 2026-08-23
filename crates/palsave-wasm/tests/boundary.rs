//! Boundary coverage: open -> edit -> export across wasm-bindgen.
//!
//! The saves are built by `palsave::synthetic` rather than loaded from `fixtures/` —
//! fixtures are gitignored (they contain SteamIDs and player names) and a browser
//! test can't read the filesystem anyway. Those builders are shared with the
//! `gen-fixtures` binary that feeds the browser suite, so there is one definition of
//! a synthetic save.
//!
//! The *format* work is verified against real saves by `palsave`'s own native fixture
//! tests; what these tests exist to prove is that handles, view models, and typed
//! errors cross the boundary intact.

use palsave::rawdata::group;
use palsave::synthetic::{synthetic_player_sav, synthetic_sav};
use palsave_wasm::open;
use wasm_bindgen_test::*;

// Runs under Node (`wasm-pack test --node`). These tests touch no DOM or Web API —
// only wasm-bindgen marshalling — so the browser runner would add a webdriver
// dependency without testing anything extra. Add
// `wasm_bindgen_test_configure!(run_in_browser)` here if a future test needs one.

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
    // The synthetic world holds one Pal, and it names this player as its owner.
    let pals = js_sys::Array::from(&get(&detail, "pals"));
    assert_eq!(pals.length(), 1);
    assert_eq!(
        get(&pals.get(0), "character_id").as_string().unwrap(),
        "Lamball"
    );
    assert_eq!(get(&pals.get(0), "talent_hp").as_f64().unwrap(), 70.0);
    assert_eq!(
        js_sys::Array::from(&handle.pals_of(&uid).expect("palsOf")).length(),
        1
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
    assert_eq!(slots.length(), 2);

    let slot = slots.get(0);
    assert_eq!(get(&slot, "static_id").as_string().unwrap(), "ClothArmor");
    assert_eq!(get(&slot, "count").as_f64().unwrap(), 5.0);
    // The DynamicItemSaveData join, all the way across the boundary. The slot's
    // DynamicId is non-zero in the fixture precisely so this can't pass by returning
    // the "no per-instance state" null.
    assert_eq!(get(&slot, "durability").as_f64().unwrap(), 150.0);

    // The second slot is an ordinary stack with no per-instance state — the common case
    // in a real save, and the one that used to be untestable because every field in the
    // fixture was populated. An absent Option must cross as `null`, not `undefined`:
    // `save-types.ts` declares these fields `T | null`, and a UI guard comparing against
    // null let `undefined` through and crashed the whole screen.
    let plain = slots.get(1);
    assert_eq!(get(&plain, "static_id").as_string().unwrap(), "Wood");
    assert_eq!(get(&plain, "count").as_f64().unwrap(), 63.0);
    assert!(
        get(&plain, "durability").is_null(),
        "an absent Option must serialize as null, not undefined"
    );
    assert!(get(&plain, "ammo_static_id").is_null());
    assert!(get(&plain, "egg_character_id").is_null());

    // Detaching really removes it.
    handle.detach_player_save(&uid);
    assert_eq!(
        js_sys::Array::from(&handle.attached_players().unwrap()).length(),
        0
    );
}

/// The Pal-box join across the boundary, including the third hop: a slot's instance id
/// must come back as a whole Pal, not just an id.
#[wasm_bindgen_test]
fn attaching_a_player_save_resolves_their_pal_storage() {
    let mut handle = open(&synthetic_sav()).expect("open level");
    let uid = handle
        .attach_player_save(&synthetic_player_sav())
        .expect("attachPlayerSave");

    let storage = handle.player_pal_storage(&uid).expect("playerPalStorage");
    assert_eq!(get(&storage, "player_uid").as_string().unwrap(), uid);

    let containers = js_sys::Array::from(&get(&storage, "containers"));
    assert_eq!(containers.length(), 2, "party and storage");

    // PAL_CONTAINER_KINDS order: party first, then the box.
    let party = containers.get(0);
    assert_eq!(get(&party, "kind").as_string().unwrap(), "party");
    assert_eq!(get(&party, "slot_count").as_f64().unwrap(), 5.0);

    let box_ = containers.get(1);
    assert_eq!(get(&box_, "kind").as_string().unwrap(), "storage");
    assert_eq!(get(&box_, "slot_count").as_f64().unwrap(), 960.0);
    assert!(!get(&box_, "missing").as_bool().unwrap());

    let slots = js_sys::Array::from(&get(&box_, "slots"));
    assert_eq!(slots.length(), 1);
    let slot = slots.get(0);
    assert_eq!(get(&slot, "slot_index").as_f64().unwrap(), 0.0);

    // The join resolved to the actual Pal, not merely to an id string.
    let pal = get(&slot, "pal");
    assert!(!pal.is_null(), "slot did not resolve to a Pal");
    assert_eq!(get(&pal, "character_id").as_string().unwrap(), "Lamball");
    assert_eq!(get(&pal, "level").as_f64().unwrap(), 12.0);
    assert_eq!(
        get(&pal, "instance_id").as_string().unwrap(),
        get(&slot, "instance_id").as_string().unwrap()
    );
}

/// Reading a Pal box needs the player's own save, exactly like reading an inventory.
#[wasm_bindgen_test]
fn pal_storage_without_an_attached_player_save_is_refused() {
    let handle = open(&synthetic_sav()).expect("open");
    let err = handle
        .player_pal_storage("00000000000000000000000000000001")
        .unwrap_err();
    assert_eq!(
        get(&err, "code").as_string().unwrap(),
        "player_save_not_attached"
    );
}

/// The migration survey across the boundary, between two synthetic worlds.
///
/// WORLD_B shares WORLD_A's Pal instance id on purpose — that is the real behaviour
/// found in the fixture corpus, where two unrelated worlds turned out to contain the
/// same instance id. Without the overlap this test would pass against a survey that
/// detects nothing.
#[wasm_bindgen_test]
fn migration_plan_crosses_the_boundary_and_reports_collisions() {
    use palsave::synthetic::{WORLD_B, synthetic_player_sav_for, synthetic_sav_for};

    // Destination is the open save; source is attached alongside it.
    let mut handle = open(&synthetic_sav()).expect("open destination");

    // Asking before attaching a source is refused, not answered with an empty plan.
    let err = handle.migration_plan("whatever").unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "no_source_world");

    handle
        .attach_source_world(&synthetic_sav_for(&WORLD_B))
        .expect("attachSourceWorld");

    let listed = js_sys::Array::from(&handle.source_players().unwrap());
    assert_eq!(listed.length(), 1);
    let uid = listed.get(0).as_string().unwrap();
    assert_eq!(uid, "00000000000000000000000000000002");

    // The source player's own save is still required — container ids live there.
    let err = handle.migration_plan(&uid).unwrap_err();
    assert_eq!(
        get(&err, "code").as_string().unwrap(),
        "player_save_not_attached"
    );

    let attached = handle
        .attach_source_player(&synthetic_player_sav_for(&WORLD_B))
        .expect("attachSourcePlayer");
    assert_eq!(attached, uid);

    let plan = handle.migration_plan(&uid).expect("migrationPlan");
    assert_eq!(get(&plan, "player_uid").as_string().unwrap(), uid);
    assert_eq!(get(&plan, "pal_count").as_f64().unwrap(), 1.0);
    assert_eq!(get(&plan, "item_container_count").as_f64().unwrap(), 1.0);
    assert_eq!(get(&plan, "pal_container_count").as_f64().unwrap(), 2.0);
    assert_eq!(get(&plan, "dynamic_item_count").as_f64().unwrap(), 1.0);
    // The player, their Pal, one item container, two Pal containers, one dynamic item.
    assert_eq!(get(&plan, "row_count").as_f64().unwrap(), 6.0);

    // WORLD_B's player uid differs from WORLD_A's, so the *only* blocking collision
    // should be the deliberately shared Pal instance id.
    assert_eq!(get(&plan, "blocking_count").as_f64().unwrap(), 1.0);
    let conflicts = js_sys::Array::from(&get(&plan, "conflicts"));
    let codes: Vec<String> = (0..conflicts.length())
        .map(|i| get(&conflicts.get(i), "code").as_string().unwrap())
        .collect();
    assert!(codes.contains(&"pal_instance_exists".to_string()));
    // The guilds differ too, so the migrated player's guild goes dangling.
    assert!(codes.contains(&"guild_missing".to_string()));
    assert!(
        !codes.contains(&"player_exists".to_string()),
        "WORLD_B's player uid differs; a player collision here means uids are being \
         compared wrongly"
    );

    // Dropping the source frees the second world and makes plans unavailable again.
    handle.clear_source();
    assert_eq!(
        js_sys::Array::from(&handle.source_players().unwrap()).length(),
        0
    );
    let err = handle.migration_plan(&uid).unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "no_source_world");
}

/// A player save handed in where a world is expected must be refused clearly.
#[wasm_bindgen_test]
fn a_player_save_is_rejected_as_a_source_world() {
    let mut handle = open(&synthetic_sav()).expect("open");
    let err = handle
        .attach_source_world(&synthetic_player_sav())
        .unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "not_a_level_save");
}

/// The diagnostic report is meant to be shareable, so its "no personal data" claim
/// is enforced here rather than trusted. The synthetic save deliberately contains a
/// player nickname, a guild name and a uid; none may appear in the report.
#[wasm_bindgen_test]
fn diagnostic_report_carries_no_personal_data() {
    let handle = open(&synthetic_sav()).expect("open");
    let report = handle.diagnostic_report().expect("diagnosticReport");

    let json = js_sys::JSON::stringify(&report)
        .expect("report serializes")
        .as_string()
        .unwrap();

    // Names the synthetic save is known to contain: a player nickname, a guild name,
    // an item id, a Pal species.
    for secret in ["Tester", "Original Name", "ClothArmor", "Lamball"] {
        assert!(
            !json.contains(secret),
            "diagnostic report leaked {secret:?}: {json}"
        );
    }

    // Every uid is 32 hex characters; a run that long means one got through.
    let bytes: Vec<char> = json.chars().collect();
    let mut run = 0usize;
    for c in bytes {
        if c.is_ascii_hexdigit() {
            run += 1;
            assert!(run < 32, "diagnostic report contains a uid-shaped hex run");
        } else {
            run = 0;
        }
    }

    // It still has to be useful.
    assert!(json.contains("engine_version"));
    assert!(json.contains("worldSaveData"));
}

/// Editing across the boundary, and the guards that keep a bad edit out of the save.
#[wasm_bindgen_test]
fn pal_stat_edits_cross_the_boundary() {
    let mut handle = open(&synthetic_sav()).expect("open");

    // An unknown stat name is refused rather than silently defaulted.
    let err = handle
        .set_pal_stat("whatever", "not_a_stat", 5.0)
        .unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "unknown_stat");

    // Out-of-range is refused before anything is written.
    let err = handle
        .set_pal_stat("whatever", "talent_hp", 9999.0)
        .unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "value_out_of_range");

    // An instance id no Pal has is refused rather than matching something else.
    let err = handle
        .set_pal_stat("whatever", "talent_hp", 50.0)
        .unwrap_err();
    assert_eq!(get(&err, "code").as_string().unwrap(), "player_not_found");

    // The world's one Pal is editable, and the edit lands on that Pal.
    let players = js_sys::Array::from(&handle.list_players().unwrap());
    let owner = get(&players.get(0), "uid").as_string().unwrap();
    let pals = js_sys::Array::from(&handle.pals_of(&owner).unwrap());
    let pal_id = get(&pals.get(0), "instance_id").as_string().unwrap();
    handle
        .set_pal_stat(&pal_id, "talent_hp", 91.0)
        .expect("setPalStat");
    let pals = js_sys::Array::from(&handle.pals_of(&owner).unwrap());
    assert_eq!(get(&pals.get(0), "talent_hp").as_f64().unwrap(), 91.0);

    // The player's own level is editable, and the change survives an export/reopen.
    let players = js_sys::Array::from(&handle.list_players().unwrap());
    let uid = get(&players.get(0), "uid").as_string().unwrap();
    handle
        .set_player_stat(&uid, "level", 42.0)
        .expect("setPlayerStat");

    let exported = handle.export().expect("export");
    let reopened = open(&exported).expect("reopen");
    let after = js_sys::Array::from(&reopened.list_players().unwrap());
    assert_eq!(get(&after.get(0), "level").as_f64().unwrap(), 42.0);
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
