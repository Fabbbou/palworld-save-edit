//! Round-trip tests against real save files in fixtures/ (gitignored). If the
//! directory has no fixtures, these no-op rather than fail, so this file works in
//! CI too — it just won't exercise anything there.

use std::path::{Path, PathBuf};

fn fixture_paths() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "sav") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    walk(&root, &mut out);
    out
}

#[test]
fn container_round_trips_on_every_fixture() {
    let fixtures = fixture_paths();
    if fixtures.is_empty() {
        eprintln!("no fixtures found, skipping");
        return;
    }
    for path in fixtures {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let container =
            palsave::container::decode(&bytes).unwrap_or_else(|e| panic!("decode {path:?}: {e}"));

        let re_encoded = palsave::container::encode(&container.gvas, &container);
        let re_decoded = palsave::container::decode(&re_encoded)
            .unwrap_or_else(|e| panic!("re-decode {path:?}: {e}"));

        assert_eq!(
            re_decoded.gvas, container.gvas,
            "{path:?}: decompressed GVAS bytes changed across a decode -> encode -> decode cycle"
        );
    }
}

#[test]
fn gvas_parses_and_round_trips_on_every_fixture() {
    let fixtures = fixture_paths();
    if fixtures.is_empty() {
        eprintln!("no fixtures found, skipping");
        return;
    }
    for path in fixtures {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let container =
            palsave::container::decode(&bytes).unwrap_or_else(|e| panic!("decode {path:?}: {e}"));

        let file = palsave::gvas::GvasFile::parse(&container.gvas)
            .unwrap_or_else(|e| panic!("gvas parse {path:?}: {e}"));

        eprintln!(
            "{path:?}: engine {}.{}.{}, save_game_type {:?}, {} top-level properties",
            file.header.engine_version_major,
            file.header.engine_version_minor,
            file.header.engine_version_patch,
            file.save_game_type,
            file.properties.len(),
        );

        assert_eq!(
            file.write(),
            container.gvas,
            "{path:?}: GVAS round-trip (parse -> write) was not byte-identical"
        );
    }
}

/// Cross-checks our header/tag-walking assumptions against uesave-rs (trumank/
/// uesave-rs, MIT), an independent, actively maintained generic UE GVAS parser named
/// as the differential-test oracle in the project plan. uesave-rs doesn't know about
/// Palworld-specific struct types, so it's run in best-effort mode (unrecognized
/// properties fall back to raw bytes instead of hard-erroring) — a hard error here
/// would mean our two implementations disagree about the *generic* UE layer (offsets,
/// tag shape, engine-version gating), which is the only thing this test is checking.
#[test]
fn uesave_oracle_parses_every_fixture() {
    let fixtures = fixture_paths();
    if fixtures.is_empty() {
        eprintln!("no fixtures found, skipping");
        return;
    }
    for path in fixtures {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
        let container =
            palsave::container::decode(&bytes).unwrap_or_else(|e| panic!("decode {path:?}: {e}"));

        let save = uesave::SaveReader::new()
            .error_to_raw(true)
            .read(std::io::Cursor::new(&container.gvas))
            .unwrap_or_else(|e| panic!("uesave failed to parse {path:?}: {e}"));

        eprintln!(
            "{path:?}: uesave read {} top-level properties, save_game_type {:?}",
            save.root.properties.0.len(),
            save.root.save_game_type,
        );

        assert_eq!(
            save.root.save_game_type,
            palsave::gvas::GvasFile::parse(&container.gvas)
                .unwrap()
                .save_game_type,
            "{path:?}: our save_game_type disagrees with uesave-rs's"
        );
    }
}

/// Materializes `.worldSaveData.GroupSaveDataMap` from a real Level.sav — this is
/// Phase 3's first target path. Confirms the Map/Struct/Guid decode path in
/// `gvas::value` works end to end on real data, not just synthetic fixtures: each
/// group's key is a bare Guid (the one case our un-hinted Map-key default happens to
/// get right — see the comment on `TagExtra::Map` in gvas/value.rs), and each value
/// is a small property list containing a "RawData" byte blob, the game-specific data
/// Phase 3's decoders exist to unpack.
#[test]
fn materializes_group_save_data_map_on_level_sav() {
    use palsave::gvas::value::{StructValue, Value};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();

    let world_save_data_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .expect("worldSaveData");
    let Value::Struct(StructValue::Properties(world_props)) =
        file.materialize(world_save_data_idx).unwrap()
    else {
        panic!("worldSaveData did not materialize as a property list");
    };

    let group_map_entry = world_props
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .expect("GroupSaveDataMap");
    let group_map = palsave::gvas::value::materialize_property(
        &container.gvas,
        group_map_entry,
        file.header.engine_version_major,
        file.header.has_property_guid(),
        "worldSaveData.GroupSaveDataMap",
    )
    .unwrap();

    let Value::Map(entries) = group_map else {
        panic!("GroupSaveDataMap did not materialize as a Map (fell back to Raw?)");
    };
    assert!(!entries.is_empty(), "expected at least one guild/group");

    for (key, value) in &entries {
        assert!(
            matches!(key, Value::Struct(StructValue::Guid(_))),
            "group key should be a bare Guid, got {key:?}"
        );
        let Value::Struct(StructValue::Properties(fields)) = value else {
            panic!("group value should be a property list, got {value:?}");
        };
        assert!(
            fields.iter().any(|f| f.name == "RawData"),
            "expected a RawData field among {:?}",
            fields.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }
}

/// Phase 3's gate: each RawData decoder round-trips independently. Extracts every
/// real guild/org from `.worldSaveData.GroupSaveDataMap` in Level.sav (GroupType +
/// RawData bytes, both already materialized by the generic GVAS layer), runs them
/// through `rawdata::group::decode`/`encode`, and asserts the re-encoded bytes are
/// byte-identical to the originals — for every group in the fixture, not just some.
#[test]
fn group_rawdata_round_trips_on_level_sav() {
    use palsave::gvas::value::{StructValue, Value};
    use palsave::rawdata::group;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();

    let world_save_data_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(world_props)) =
        file.materialize(world_save_data_idx).unwrap()
    else {
        panic!("worldSaveData did not materialize as a property list");
    };
    let group_map_entry = world_props
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .unwrap();
    let group_map = palsave::gvas::value::materialize_property(
        &container.gvas,
        group_map_entry,
        file.header.engine_version_major,
        file.header.has_property_guid(),
        "worldSaveData.GroupSaveDataMap",
    )
    .unwrap();
    let Value::Map(entries) = group_map else {
        panic!("GroupSaveDataMap did not materialize as a Map")
    };

    let mut decoded_count = 0;
    for (_key, value) in &entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            panic!("expected a property list")
        };

        let group_type_entry = fields
            .iter()
            .find(|f| f.name == "GroupType")
            .expect("GroupType");
        let group_type = palsave::gvas::value::materialize_property(
            &container.gvas,
            group_type_entry,
            file.header.engine_version_major,
            file.header.has_property_guid(),
            "worldSaveData.GroupSaveDataMap.Value.GroupType",
        )
        .unwrap();
        let Value::Enum(group_type) = group_type else {
            panic!("GroupType wasn't an Enum")
        };
        let group_type = group_type.display_lossy();

        let raw_data_entry = fields
            .iter()
            .find(|f| f.name == "RawData")
            .expect("RawData");
        let raw_data = palsave::gvas::value::materialize_property(
            &container.gvas,
            raw_data_entry,
            file.header.engine_version_major,
            file.header.has_property_guid(),
            "worldSaveData.GroupSaveDataMap.Value.RawData",
        )
        .unwrap();
        let Value::Bytes(raw_bytes) = raw_data else {
            panic!("RawData wasn't a byte blob")
        };

        let decoded = group::decode(&raw_bytes, &group_type)
            .unwrap_or_else(|e| panic!("group::decode failed for group_type {group_type:?}: {e}"));
        let re_encoded = group::encode(&decoded);
        assert_eq!(
            re_encoded, raw_bytes,
            "group_type {group_type:?}: RawData round-trip was not byte-identical"
        );
        decoded_count += 1;
    }

    eprintln!(
        "decoded and round-tripped {decoded_count}/{} groups from Level.sav",
        entries.len()
    );
    assert_eq!(decoded_count, entries.len());
}

/// Phase 3's second target: every real player/Pal in `.worldSaveData.
/// CharacterSaveParameterMap`'s RawData, decoded and round-tripped byte-identically.
/// The gvas::hints table (also ported from oMaN-Rod/uesave-rs) is what makes this
/// map materialize as a Map at all — its key is a small property list, not the bare
/// Guid our un-hinted default assumes, so before the hint table this fell back to
/// `Value::Raw` entirely (see the comment removed from this file in the same change
/// that added gvas/hints.rs).
#[test]
fn character_rawdata_round_trips_on_level_sav() {
    use palsave::gvas::value::{StructValue, Value};
    use palsave::rawdata::character;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();
    let has_property_guid = file.header.has_property_guid();

    let world_save_data_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(world_props)) =
        file.materialize(world_save_data_idx).unwrap()
    else {
        panic!("worldSaveData did not materialize as a property list");
    };
    let map_entry = world_props
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .unwrap();
    let map = palsave::gvas::value::materialize_property(
        &container.gvas,
        map_entry,
        file.header.engine_version_major,
        has_property_guid,
        "worldSaveData.CharacterSaveParameterMap",
    )
    .unwrap();
    let Value::Map(entries) = map else {
        panic!("CharacterSaveParameterMap did not materialize as a Map")
    };
    assert!(
        entries.len() > 100,
        "expected the ~137 characters seen when this test was written, got {}",
        entries.len()
    );

    let mut decoded_count = 0;
    for (_key, value) in &entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            panic!("expected a property list")
        };

        let raw_data_entry = fields
            .iter()
            .find(|f| f.name == "RawData")
            .expect("RawData");
        let raw_data = palsave::gvas::value::materialize_property(
            &container.gvas,
            raw_data_entry,
            file.header.engine_version_major,
            has_property_guid,
            "worldSaveData.CharacterSaveParameterMap.Value.RawData",
        )
        .unwrap();
        let Value::Bytes(raw_bytes) = raw_data else {
            panic!("RawData wasn't a byte blob")
        };

        let decoded = character::decode(&raw_bytes, has_property_guid)
            .unwrap_or_else(|e| panic!("character::decode failed: {e}"));
        let re_encoded = character::encode(&raw_bytes, &decoded);
        assert_eq!(
            re_encoded, raw_bytes,
            "character RawData round-trip was not byte-identical"
        );
        decoded_count += 1;
    }

    eprintln!(
        "decoded and round-tripped {decoded_count}/{} characters from Level.sav",
        entries.len()
    );
    assert_eq!(decoded_count, entries.len());
}

/// Phase 3's third target: `.worldSaveData.ItemContainerSaveData` — inventories.
/// Two RawData shapes at two paths, both exercised here across every real container
/// in Level.sav: the container's own permissions blob (`.Value.RawData`), and each
/// inventory slot's item/count blob (`.Value.Slots[].RawData`).
#[test]
fn item_container_rawdata_round_trips_on_level_sav() {
    use palsave::gvas::value::{StructValue, Value};
    use palsave::rawdata::item_container;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();
    let has_property_guid = file.header.has_property_guid();
    let engine_major = file.header.engine_version_major;

    let world_save_data_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(world_props)) =
        file.materialize(world_save_data_idx).unwrap()
    else {
        panic!("worldSaveData did not materialize as a property list");
    };
    let map_entry = world_props
        .iter()
        .find(|p| p.name == "ItemContainerSaveData")
        .unwrap();
    let map = palsave::gvas::value::materialize_property(
        &container.gvas,
        map_entry,
        engine_major,
        has_property_guid,
        "worldSaveData.ItemContainerSaveData",
    )
    .unwrap();
    let Value::Map(entries) = map else {
        panic!("ItemContainerSaveData did not materialize as a Map")
    };
    assert!(
        entries.len() > 1000,
        "expected the ~1488 containers seen when this test was written, got {}",
        entries.len()
    );

    let mut containers_checked = 0;
    let mut slots_checked = 0;

    for (_key, value) in &entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            panic!("expected a property list")
        };

        // The container's own permissions blob.
        let raw_data_entry = fields
            .iter()
            .find(|f| f.name == "RawData")
            .expect("RawData");
        let raw_data = palsave::gvas::value::materialize_property(
            &container.gvas,
            raw_data_entry,
            engine_major,
            has_property_guid,
            "worldSaveData.ItemContainerSaveData.Value.RawData",
        )
        .unwrap();
        let Value::Bytes(raw_bytes) = raw_data else {
            panic!("container RawData wasn't a byte blob")
        };
        let decoded = item_container::decode_container(&raw_bytes)
            .unwrap_or_else(|e| panic!("decode_container failed: {e}"));
        assert_eq!(
            item_container::encode_container(&decoded),
            raw_bytes,
            "container RawData round-trip was not byte-identical"
        );
        containers_checked += 1;

        // Each inventory slot's item/count blob.
        let slots_entry = fields.iter().find(|f| f.name == "Slots").expect("Slots");
        let slots = palsave::gvas::value::materialize_property(
            &container.gvas,
            slots_entry,
            engine_major,
            has_property_guid,
            "worldSaveData.ItemContainerSaveData.Value.Slots",
        )
        .unwrap();
        let Value::Array(slot_items) = slots else {
            panic!("Slots wasn't an Array")
        };

        for slot in &slot_items {
            let Value::Struct(StructValue::Properties(slot_fields)) = slot else {
                panic!("slot wasn't a property list")
            };
            let slot_raw_entry = slot_fields
                .iter()
                .find(|f| f.name == "RawData")
                .expect("slot RawData");
            let slot_raw = palsave::gvas::value::materialize_property(
                &container.gvas,
                slot_raw_entry,
                engine_major,
                has_property_guid,
                "worldSaveData.ItemContainerSaveData.Value.Slots.RawData",
            )
            .unwrap();
            let Value::Bytes(slot_bytes) = slot_raw else {
                panic!("slot RawData wasn't a byte blob")
            };
            let decoded_slot = item_container::decode_slot(&slot_bytes)
                .unwrap_or_else(|e| panic!("decode_slot failed: {e}"));
            assert_eq!(
                item_container::encode_slot(&decoded_slot),
                slot_bytes,
                "slot RawData round-trip was not byte-identical"
            );
            slots_checked += 1;
        }
    }

    eprintln!(
        "round-tripped {containers_checked} containers and {slots_checked} slots from Level.sav"
    );
    assert_eq!(containers_checked, entries.len());
    assert!(
        slots_checked > 0,
        "expected at least one non-empty inventory slot"
    );
}

/// Collects every `worldSaveData` child property's name and raw bytes, so a
/// before/after comparison can prove an edit touched exactly one of them. Absolute
/// offsets shift when an edit changes a length, so this compares *content*, which is
/// the invariant that actually matters.
fn world_save_data_children(gvas: &[u8]) -> Vec<(String, Vec<u8>)> {
    use palsave::gvas::value::{StructValue, Value};

    let file = palsave::gvas::GvasFile::parse(gvas).unwrap();
    let idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(children)) = file.materialize(idx).unwrap() else {
        panic!("worldSaveData did not materialize as a property list");
    };
    children
        .iter()
        .map(|c| (c.name.clone(), gvas[c.span.clone()].to_vec()))
        .collect()
}

/// Phase 4's gate: renaming one guild leaves every unrelated byte identical.
///
/// Renames the single real `EPalGroupType::Guild` in Level.sav to a name of a
/// *different length* (so every enclosing `size` field must be fixed up — a
/// same-length rename would pass even with the fixup logic entirely missing), then
/// asserts: the edited buffer re-parses into an exact partition of itself, the new
/// name reads back, and every sibling of `GroupSaveDataMap` under `worldSaveData` is
/// byte-for-byte unchanged.
#[test]
fn renaming_a_guild_leaves_every_unrelated_byte_identical() {
    use palsave::edit;
    use palsave::gvas::primitives::FString;
    use palsave::gvas::value::{StructValue, Value};
    use palsave::rawdata::group;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let source = &container.gvas;
    let file = palsave::gvas::GvasFile::parse(source).unwrap();
    let has_property_guid = file.header.has_property_guid();
    let engine_major = file.header.engine_version_major;

    // Descend worldSaveData -> GroupSaveDataMap -> (the Guild entry's) RawData,
    // keeping every ancestor: that chain is what the splice engine needs in order to
    // fix up each enclosing `size` field.
    let world_idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let world_entry = file.properties[world_idx].clone();
    let Value::Struct(StructValue::Properties(world_children)) =
        file.materialize(world_idx).unwrap()
    else {
        panic!("worldSaveData did not materialize as a property list");
    };
    let map_entry = world_children
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .unwrap()
        .clone();
    let map = palsave::gvas::value::materialize_property(
        source,
        &map_entry,
        engine_major,
        has_property_guid,
        "worldSaveData.GroupSaveDataMap",
    )
    .unwrap();
    let Value::Map(entries) = map else {
        panic!("GroupSaveDataMap wasn't a Map")
    };

    let mut guild_raw_entry = None;
    for (_key, value) in &entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            continue;
        };
        let gt_entry = fields.iter().find(|f| f.name == "GroupType").unwrap();
        let gt = palsave::gvas::value::materialize_property(
            source,
            gt_entry,
            engine_major,
            has_property_guid,
            "worldSaveData.GroupSaveDataMap.Value.GroupType",
        )
        .unwrap();
        let Value::Enum(gt) = gt else { continue };
        if gt.display_lossy() == group::GUILD {
            guild_raw_entry = Some(fields.iter().find(|f| f.name == "RawData").unwrap().clone());
            break;
        }
    }
    let guild_raw_entry = guild_raw_entry.expect("no EPalGroupType::Guild in the fixture");

    // Decode the guild, rename it, re-encode.
    let raw = palsave::gvas::value::materialize_property(
        source,
        &guild_raw_entry,
        engine_major,
        has_property_guid,
        "worldSaveData.GroupSaveDataMap.Value.RawData",
    )
    .unwrap();
    let Value::Bytes(raw_bytes) = raw else {
        panic!("RawData wasn't a byte blob")
    };
    let mut decoded = group::decode(&raw_bytes, group::GUILD).unwrap();

    const NEW_NAME: &str = "A Deliberately Much Longer Guild Name";
    let group::GroupVariant::Guild(guild) = &mut decoded.data else {
        panic!("not a Guild variant")
    };
    let original_name = guild.guild_name.display_lossy();
    assert_ne!(original_name, NEW_NAME);
    guild.guild_name = FString::Ascii {
        content: NEW_NAME.as_bytes().to_vec(),
        trailing: vec![0],
    };

    let new_blob = group::encode(&decoded);
    assert_ne!(
        new_blob.len(),
        raw_bytes.len(),
        "test needs a length-changing edit to exercise size fixups"
    );

    // Splice it in, fixing up every enclosing size field.
    let splices = edit::replace_property_value(
        source,
        &[&world_entry, &map_entry, &guild_raw_entry],
        edit::byte_array_value(&new_blob),
        has_property_guid,
    )
    .unwrap();
    let edited = splices.apply(source).unwrap();

    // Structural verification: every size field must agree with the real layout.
    edit::verify_reparses(&edited).expect("edited buffer failed structural verification");

    let delta = new_blob.len() as i64 - raw_bytes.len() as i64;
    assert_eq!(
        edited.len() as i64 - source.len() as i64,
        delta,
        "buffer grew by something other than the edit delta"
    );

    // The rename actually landed.
    let edited_file = palsave::gvas::GvasFile::parse(&edited).unwrap();
    let edited_world_idx = edited_file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(edited_children)) =
        edited_file.materialize(edited_world_idx).unwrap()
    else {
        panic!()
    };
    let edited_map_entry = edited_children
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .unwrap();
    let edited_map = palsave::gvas::value::materialize_property(
        &edited,
        edited_map_entry,
        engine_major,
        has_property_guid,
        "worldSaveData.GroupSaveDataMap",
    )
    .unwrap();
    let Value::Map(edited_entries) = edited_map else {
        panic!("GroupSaveDataMap wasn't a Map after edit")
    };
    assert_eq!(
        edited_entries.len(),
        entries.len(),
        "edit changed the number of groups"
    );

    let mut found_new_name = false;
    for (_key, value) in &edited_entries {
        let Value::Struct(StructValue::Properties(fields)) = value else {
            continue;
        };
        let gt_entry = fields.iter().find(|f| f.name == "GroupType").unwrap();
        let gt = palsave::gvas::value::materialize_property(
            &edited,
            gt_entry,
            engine_major,
            has_property_guid,
            "worldSaveData.GroupSaveDataMap.Value.GroupType",
        )
        .unwrap();
        let Value::Enum(gt) = gt else { continue };
        if gt.display_lossy() != group::GUILD {
            continue;
        }
        let rd_entry = fields.iter().find(|f| f.name == "RawData").unwrap();
        let rd = palsave::gvas::value::materialize_property(
            &edited,
            rd_entry,
            engine_major,
            has_property_guid,
            "worldSaveData.GroupSaveDataMap.Value.RawData",
        )
        .unwrap();
        let Value::Bytes(rd) = rd else { panic!() };
        let redecoded =
            group::decode(&rd, group::GUILD).expect("edited guild RawData no longer decodes");
        let group::GroupVariant::Guild(g) = &redecoded.data else {
            panic!()
        };
        assert_eq!(g.guild_name.display_lossy(), NEW_NAME);
        found_new_name = true;
    }
    assert!(found_new_name, "renamed guild not found after edit");

    // THE GATE: every sibling of GroupSaveDataMap is byte-for-byte untouched.
    let before = world_save_data_children(source);
    let after = world_save_data_children(&edited);
    assert_eq!(
        before.len(),
        after.len(),
        "edit added or removed a worldSaveData child"
    );

    let mut changed = Vec::new();
    for ((name_before, bytes_before), (name_after, bytes_after)) in before.iter().zip(after.iter())
    {
        assert_eq!(name_before, name_after, "worldSaveData children reordered");
        if bytes_before != bytes_after {
            changed.push(name_after.clone());
        }
    }
    assert_eq!(
        changed,
        vec!["GroupSaveDataMap".to_string()],
        "expected exactly GroupSaveDataMap to change, got {changed:?}"
    );

    eprintln!(
        "renamed guild {original_name:?} -> {NEW_NAME:?} ({delta:+} bytes); \
         {} worldSaveData children, only GroupSaveDataMap changed",
        after.len()
    );
}

/// The task-level guild API (`palsave::guilds`) end to end on a real save: list,
/// detail, rename, and confirm the rename survives a full container re-encode and
/// re-open — i.e. the path the wasm `export()` will actually take.
#[test]
fn guild_api_round_trips_on_level_sav() {
    use palsave::guilds;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();

    let summaries = guilds::list(&container.gvas).unwrap();
    assert!(!summaries.is_empty(), "expected at least one group");
    for s in &summaries {
        eprintln!(
            "{} [{}] name={:?} members={} camp_level={} pals={}",
            s.id, s.group_type, s.name, s.member_count, s.base_camp_level, s.pal_count
        );
    }

    let guild = summaries
        .iter()
        .find(|s| s.group_type == palsave::rawdata::group::GUILD)
        .expect("no EPalGroupType::Guild in fixture");

    let detail = guilds::detail(&container.gvas, &guild.id).unwrap();
    assert_eq!(detail.summary.id, guild.id);
    assert!(!detail.members.is_empty(), "guild should have members");
    assert!(detail.admin_player_uid.is_some());

    // Rename, then verify through a full decompress -> edit -> recompress -> reopen
    // cycle, which is exactly what the wasm export path does.
    const NEW_NAME: &str = "Renamed By palsave";
    let edited_gvas = guilds::set_name(&container.gvas, &guild.id, NEW_NAME).unwrap();

    let sav = palsave::container::encode(&edited_gvas, &container);
    let reopened = palsave::container::decode(&sav).unwrap();
    assert_eq!(
        reopened.gvas, edited_gvas,
        "container round-trip changed the GVAS payload"
    );

    let after = guilds::list(&reopened.gvas).unwrap();
    assert_eq!(
        after.len(),
        summaries.len(),
        "rename changed the number of groups"
    );
    let renamed = after.iter().find(|s| s.id == guild.id).unwrap();
    assert_eq!(renamed.name, NEW_NAME);
    assert_eq!(
        renamed.member_count, guild.member_count,
        "rename disturbed the member roster"
    );
    assert_eq!(
        renamed.pal_count, guild.pal_count,
        "rename disturbed the pal roster"
    );

    // Every other group is untouched.
    for (before, after) in summaries.iter().zip(after.iter()) {
        if before.id == guild.id {
            continue;
        }
        assert_eq!(before, after, "rename disturbed an unrelated group");
    }

    // Error paths carry stable codes.
    let err = guilds::set_name(&container.gvas, "not-a-guid", "x").unwrap_err();
    assert_eq!(err.code(), "malformed_guild_id");
    let err = guilds::detail(&container.gvas, &"0".repeat(32)).unwrap_err();
    assert_eq!(err.code(), "guild_not_found");

    eprintln!(
        "guild API: renamed {:?} -> {NEW_NAME:?}, survived container round-trip",
        guild.name
    );
}

/// The character task layer against a real save: player detection, uid convention,
/// and Pal decoding.
#[test]
fn lists_players_on_level_sav() {
    use palsave::characters;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();

    let players = characters::list_players(&container.gvas).unwrap();
    assert_eq!(players.len(), 1, "fixture has exactly one player");
    let player = &players[0];

    // The uid must render in Unreal's display convention, because that is what names
    // the player's own save file. A raw byte-order dump would print ...01000000 here
    // and silently fail to pair with Players/<uid>.sav.
    let player_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Players");
    if let Ok(entries) = std::fs::read_dir(&player_dir) {
        let stems: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "sav"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
            })
            .collect();
        assert!(
            stems.contains(&player.uid),
            "player uid {} matched none of the Players/*.sav filenames {stems:?}",
            player.uid
        );
    }

    assert!(player.nickname.is_some(), "player should have a NickName");
    assert!(player.level.unwrap_or(0) > 0, "player should have a level");
    assert!(player.pal_count > 0, "player should own Pals");

    eprintln!(
        "player {} level {:?} owns {} pals",
        player.uid, player.level, player.pal_count
    );
}

#[test]
fn lists_pals_on_level_sav() {
    use palsave::characters;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();

    let pals = characters::list_all_pals(&container.gvas).unwrap();
    let players = characters::list_players(&container.gvas).unwrap();

    // Every character is either a player or a Pal; nothing may be dropped.
    assert_eq!(
        pals.len() + players.len(),
        137,
        "fixture has 137 characters total"
    );

    // CharacterID is the species and is present on every Pal in this fixture — if a
    // future save makes it absent the decode still succeeds, but this asserts the
    // field name hasn't moved.
    assert!(
        pals.iter().all(|p| p.character_id.is_some()),
        "every Pal should decode a CharacterID"
    );
    assert!(
        pals.iter().any(|p| p.talent_hp.is_some()),
        "at least one Pal should decode IVs"
    );
    assert!(
        pals.iter().any(|p| !p.passive_skills.is_empty()),
        "at least one Pal should decode passive skills"
    );
}

/// The load-bearing cross-check: the player↔Pal ownership link, validated against an
/// independently-decoded structure. `pals_of` reads `OwnerPlayerUId` out of each
/// Pal's RawData; the guild's `individual_character_handle_ids` is a completely
/// separate list in a different map. If the uid convention or the owner lookup were
/// wrong, `pals_of` would silently return zero and only this test would notice.
#[test]
fn pals_of_agrees_with_independent_sources() {
    use palsave::characters;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();

    let players = characters::list_players(&container.gvas).unwrap();
    let player = &players[0];

    let owned = characters::pals_of(&container.gvas, &player.uid).unwrap();
    assert!(
        !owned.is_empty(),
        "pals_of returned nothing — owner lookup is broken"
    );
    assert_eq!(
        owned.len(),
        player.pal_count,
        "pals_of disagrees with the summary's own pal_count"
    );

    // Every Pal it returned really does name this player.
    assert!(
        owned
            .iter()
            .all(|p| p.owner_player_uid.as_deref() == Some(player.uid.as_str())),
        "pals_of returned a Pal owned by someone else"
    );

    // The guild lists every character in the guild — owned Pals plus base-camp Pals
    // plus the player — so it bounds the owned count from above and must not be
    // wildly larger, which would mean owner reads are being missed.
    let guilds = palsave::guilds::list(&container.gvas).unwrap();
    if let Some(guild) = guilds
        .iter()
        .find(|g| g.group_type == palsave::rawdata::group::GUILD)
    {
        assert!(
            owned.len() <= guild.pal_count,
            "owned pals ({}) exceed the guild's character handles ({})",
            owned.len(),
            guild.pal_count
        );
        eprintln!(
            "owned {} of {} guild character handles",
            owned.len(),
            guild.pal_count
        );
    }

    // player() must agree with the standalone calls.
    let detail = characters::player(&container.gvas, &player.uid).unwrap();
    assert_eq!(detail.summary, *player);
    assert_eq!(detail.pals.len(), owned.len());

    // Error paths carry stable codes.
    assert_eq!(
        characters::player(&container.gvas, "nope")
            .unwrap_err()
            .code(),
        "malformed_uid"
    );
    assert_eq!(
        characters::player(&container.gvas, &"0".repeat(32))
            .unwrap_err()
            .code(),
        "player_not_found"
    );
}

/// Regression guard for the blob-relative span trap `gvas::nav::Cursor::rebase`
/// exists to prevent: a RawData blob's property spans index the blob, not the save.
/// Materializing with the wrong buffer yields no error, just wrong bytes — so assert
/// the values are actually sane, not merely that the call succeeded.
#[test]
fn character_navigation_is_blob_relative() {
    use palsave::characters;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let pals = characters::list_all_pals(&container.gvas).unwrap();

    for pal in &pals {
        // Species names are ASCII identifiers; garbage from a wrong-buffer read would
        // not be.
        if let Some(id) = &pal.character_id {
            assert!(
                !id.is_empty() && id.chars().all(|c| c.is_ascii_graphic()),
                "implausible CharacterID {id:?} — reading from the wrong buffer?"
            );
        }
        // IVs are a 0..=100 game stat stored in a byte.
        for iv in [pal.talent_hp, pal.talent_shot, pal.talent_defense]
            .into_iter()
            .flatten()
        {
            assert!(
                (0..=100).contains(&iv),
                "IV {iv} outside the plausible 0..=100"
            );
        }
        if let Some(level) = pal.level {
            assert!((1..=100).contains(&level), "level {level} outside 1..=100");
        }
    }
}

/// Locates the fixture's player save, if one was provided.
fn player_fixture() -> Option<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Players");
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let path = e.path();
        path.extension().is_some_and(|x| x == "sav").then_some(path)
    })
}

/// The two-file join: container ids come from the player's own save, and every one
/// of them must resolve to a real entry in Level.sav's container map.
#[test]
fn player_inventory_resolves_containers_from_the_player_save() {
    use palsave::inventory;

    let level_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    let Some(player_path) = player_fixture() else {
        eprintln!("no player fixture found, skipping");
        return;
    };
    if !level_path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }

    let level = palsave::container::decode(&std::fs::read(&level_path).unwrap()).unwrap();
    let player = palsave::container::decode(&std::fs::read(&player_path).unwrap()).unwrap();

    // The uid must come from the player file itself and match its own filename.
    let uid = inventory::player_uid(&player.gvas).unwrap();
    let stem = player_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert_eq!(uid, stem, "player uid should match the save's filename");

    let inv = inventory::player_inventory(&level.gvas, &player.gvas).unwrap();
    assert_eq!(inv.player_uid, uid);
    assert!(
        !inv.containers.is_empty(),
        "player has no containers — the InventoryInfo lookup found nothing"
    );

    // Every id the player file named must exist in Level.sav. A `missing` container
    // here would mean the two files disagree, which for a matched pair is a bug.
    for container in &inv.containers {
        assert!(
            !container.missing,
            "{:?} container {} named by the player save has no entry in Level.sav",
            container.kind, container.id
        );
    }

    // At least one container should actually hold something, or the join silently
    // "worked" while returning nothing useful.
    let occupied: usize = inv.containers.iter().map(|c| c.slots.len()).sum();
    assert!(occupied > 0, "no occupied slots found across any container");

    eprintln!(
        "resolved {} containers, {occupied} occupied slots",
        inv.containers.len()
    );
    for c in &inv.containers {
        eprintln!("  {:?}: {}/{} slots", c.kind, c.slots.len(), c.slot_count);
    }
}

/// The load-bearing check. A mis-joined guid or a wrong buffer still returns `Ok`
/// with structurally valid-looking data — it's the *values* that give it away, so
/// assert they're plausible rather than merely present.
#[test]
fn inventory_slot_contents_are_plausible() {
    use palsave::inventory;

    let level_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    let Some(player_path) = player_fixture() else {
        eprintln!("no player fixture found, skipping");
        return;
    };
    if !level_path.exists() {
        return;
    }

    let level = palsave::container::decode(&std::fs::read(&level_path).unwrap()).unwrap();
    let player = palsave::container::decode(&std::fs::read(&player_path).unwrap()).unwrap();
    let inv = inventory::player_inventory(&level.gvas, &player.gvas).unwrap();

    for container in &inv.containers {
        assert!(
            container.slot_count >= 0 && container.slot_count < 10_000,
            "{:?}: implausible SlotNum {}",
            container.kind,
            container.slot_count
        );
        assert!(
            container.slots.len() <= container.slot_count as usize,
            "{:?}: {} occupied slots exceeds capacity {}",
            container.kind,
            container.slots.len(),
            container.slot_count
        );

        for slot in &container.slots {
            assert!(slot.count >= 1, "listed slot with count {}", slot.count);
            assert!(
                slot.slot_index >= 0 && slot.slot_index < container.slot_count,
                "{:?}: slot_index {} outside 0..{}",
                container.kind,
                slot.slot_index,
                container.slot_count
            );
            // Item ids are ASCII identifiers; a wrong-buffer read would not be.
            if let Some(id) = &slot.static_id {
                assert!(
                    !id.is_empty() && id.chars().all(|c| c.is_ascii_graphic()),
                    "implausible item id {id:?} — reading from the wrong buffer?"
                );
            }
        }

        // Slots must be listed in slot order for a stable UI.
        let indices: Vec<i32> = container.slots.iter().map(|s| s.slot_index).collect();
        let mut sorted = indices.clone();
        sorted.sort_unstable();
        assert_eq!(indices, sorted, "slots are not in slot_index order");
    }

    // The dynamic-item join has to survive all the way into the view model, not just
    // work in isolation — `dynamic_item_ids_resolve_or_are_zero` proves the keys match,
    // this proves `player_inventory` actually attaches what it looked up.
    let enriched: Vec<&inventory::SlotView> = inv
        .containers
        .iter()
        .flat_map(|c| c.slots.iter())
        .filter(|s| {
            s.durability.is_some() || s.ammo_static_id.is_some() || s.egg_character_id.is_some()
        })
        .collect();
    assert!(
        !enriched.is_empty(),
        "no slot carries any dynamic item state — the join is producing None everywhere"
    );
    for slot in &enriched {
        if let Some(d) = slot.durability {
            assert!(
                d.is_finite() && (0.0..=100_000.0).contains(&d),
                "implausible durability {d} on {:?}",
                slot.static_id
            );
        }
        // The `None` sentinel must have been filtered out, not shown as an item name.
        assert_ne!(slot.ammo_static_id.as_deref(), Some("None"));
    }
    eprintln!(
        "{} of {} occupied slots carry dynamic item state",
        enriched.len(),
        inv.containers.iter().map(|c| c.slots.len()).sum::<usize>()
    );
}

/// The file-type guard: dropping two Level.sav files must fail clearly rather than
/// producing an empty inventory that looks like "you own nothing".
#[test]
fn a_level_save_is_rejected_as_a_player_save() {
    use palsave::inventory;

    let level_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !level_path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let level = palsave::container::decode(&std::fs::read(&level_path).unwrap()).unwrap();

    let err = inventory::player_uid(&level.gvas).unwrap_err();
    assert_eq!(err.code(), "not_a_player_save");

    let err = inventory::player_inventory(&level.gvas, &level.gvas).unwrap_err();
    assert_eq!(err.code(), "not_a_player_save");
}

/// Phase A's gate: editing one Pal's stat changes that stat and nothing else.
///
/// The nested splice touches two buffers — the RawData blob and the save — so a
/// missed size fixup at either level corrupts everything after the edit point. This
/// asserts every *other* Pal is byte-identical, which is what catches that; a wrong
/// fixup shifts subsequent offsets and they all change at once.
#[test]
fn setting_a_pal_stat_changes_only_that_stat() {
    use palsave::characters::{self, PalStat};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        eprintln!("no Level.sav fixture found, skipping");
        return;
    }
    let container = palsave::container::decode(&std::fs::read(&path).unwrap()).unwrap();

    let before = characters::list_all_pals(&container.gvas).unwrap();
    // A Pal that actually has the field, so this tests an edit rather than a refusal.
    let target = before
        .iter()
        .find(|p| p.talent_hp.is_some() && p.talent_hp != Some(99))
        .expect("no Pal with a Talent_HP to edit");
    let original_hp = target.talent_hp.unwrap();

    let edited =
        characters::set_pal_stat(&container.gvas, &target.instance_id, PalStat::TalentHp, 99)
            .unwrap();

    // Structural: every size field still agrees with the real byte layout.
    palsave::edit::verify_reparses(&edited).expect("edited save failed verification");

    let after = characters::list_all_pals(&edited).unwrap();
    assert_eq!(after.len(), before.len(), "edit changed the number of Pals");

    let mut changed = 0;
    for (b, a) in before.iter().zip(after.iter()) {
        if b.instance_id == target.instance_id {
            assert_eq!(a.talent_hp, Some(99), "the edit did not land");
            assert_ne!(original_hp, 99, "test picked a no-op value");
            // Everything else about this Pal survives.
            assert_eq!(a.character_id, b.character_id);
            assert_eq!(a.level, b.level);
            assert_eq!(a.talent_shot, b.talent_shot);
            assert_eq!(a.talent_defense, b.talent_defense);
            assert_eq!(a.passive_skills, b.passive_skills);
            assert_eq!(a.friendship_point, b.friendship_point);
            changed += 1;
        } else {
            assert_eq!(a, b, "an unrelated Pal changed");
        }
    }
    assert_eq!(changed, 1, "expected exactly one Pal to change");

    // Unrelated subsystems are untouched.
    let guilds_before = palsave::guilds::list(&container.gvas).unwrap();
    let guilds_after = palsave::guilds::list(&edited).unwrap();
    assert_eq!(guilds_before, guilds_after, "the guild map changed");

    eprintln!(
        "Talent_HP {original_hp} -> 99 on one of {} Pals",
        before.len()
    );
}

/// A length-changing edit, which is the case that actually exercises the size fixups
/// at both nesting levels. A same-length write would pass even with them missing.
#[test]
fn setting_a_pal_nickname_handles_length_changes() {
    use palsave::characters;

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        return;
    }
    let container = palsave::container::decode(&std::fs::read(&path).unwrap()).unwrap();
    let before = characters::list_all_pals(&container.gvas).unwrap();

    let target = before
        .iter()
        .find(|p| p.nickname.is_some())
        .expect("no Pal with a NickName to edit");

    for name in ["A Considerably Longer Nickname Than Before", "Bo"] {
        let edited =
            characters::set_pal_nickname(&container.gvas, &target.instance_id, name).unwrap();
        palsave::edit::verify_reparses(&edited).expect("verification failed");

        let after = characters::list_all_pals(&edited).unwrap();
        assert_eq!(after.len(), before.len());

        let updated = after
            .iter()
            .find(|p| p.instance_id == target.instance_id)
            .unwrap();
        assert_eq!(updated.nickname.as_deref(), Some(name));

        // Length changed, so every later Pal shifted in the file — but their decoded
        // content must be identical.
        for (b, a) in before.iter().zip(after.iter()) {
            if b.instance_id != target.instance_id {
                assert_eq!(a, b, "an unrelated Pal changed after a resizing edit");
            }
        }
    }
}

#[test]
fn out_of_range_and_missing_fields_are_refused() {
    use palsave::characters::{self, PalStat};

    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/Level.sav");
    if !path.exists() {
        return;
    }
    let container = palsave::container::decode(&std::fs::read(&path).unwrap()).unwrap();
    let pals = characters::list_all_pals(&container.gvas).unwrap();
    let target = pals.iter().find(|p| p.talent_hp.is_some()).unwrap();

    // An IV the game would not accept.
    let err = characters::set_pal_stat(
        &container.gvas,
        &target.instance_id,
        PalStat::TalentHp,
        9999,
    )
    .unwrap_err();
    assert_eq!(err.code(), "value_out_of_range");

    let err = characters::set_pal_stat(&container.gvas, &target.instance_id, PalStat::Level, 0)
        .unwrap_err();
    assert_eq!(err.code(), "value_out_of_range");

    // A Pal that doesn't exist.
    let err = characters::set_pal_stat(&container.gvas, "nope", PalStat::TalentHp, 50).unwrap_err();
    assert_eq!(err.code(), "player_not_found");

    // A field this Pal genuinely lacks is refused, never inserted.
    if let Some(no_nick) = pals.iter().find(|p| p.nickname.is_none()) {
        let err =
            characters::set_pal_nickname(&container.gvas, &no_nick.instance_id, "X").unwrap_err();
        assert_eq!(err.code(), "field_not_present");
    }
}

/// Every `Level.sav` under `fixtures/`, at any depth. Most tests here are pinned to
/// one save because they assert exact counts; this exists for the checks that should
/// hold for *any* world.
fn level_fixtures() -> Vec<PathBuf> {
    fixture_paths()
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "Level.sav"))
        .collect()
}

/// Runs all three RawData decoders over every world available, asserting each blob
/// re-encodes byte-identically.
///
/// The single-fixture tests above prove the decoders work on *one* world. That world
/// happened to have `base_ids`, `guild_markers`, `role_permissions` and
/// `guild_chest_allowed_roles` all empty, so those element encoders were exercised
/// only by synthetic tests — a wrong element size would have round-tripped an empty
/// list perfectly and corrupted a populated one. This is the test that would catch
/// that, and it only has teeth when a second world is present.
#[test]
fn rawdata_decoders_round_trip_on_every_level_save() {
    use palsave::gvas::nav::find;
    use palsave::gvas::value::Value;
    use palsave::rawdata::{character, character_container, dynamic_item, group, item_container};

    let levels = level_fixtures();
    if levels.is_empty() {
        eprintln!("no Level.sav fixtures found, skipping");
        return;
    }

    for path in levels {
        let container = palsave::container::decode(&std::fs::read(&path).unwrap())
            .unwrap_or_else(|e| panic!("decode {path:?}: {e}"));
        let gvas = &container.gvas;
        let label = path.display();

        // Guilds and organizations.
        let map = palsave::world::open_map(gvas, "GroupSaveDataMap").unwrap();
        let mut populated_base_ids = 0usize;
        let mut populated_role_perms = 0usize;
        for entry in &map.entries {
            let (Some(gt), Some(rd)) = (
                find(&entry.fields, "GroupType"),
                find(&entry.fields, "RawData"),
            ) else {
                continue;
            };
            let Value::Enum(gt) = map.cursor.materialize(gt).unwrap() else {
                continue;
            };
            let gt = gt.display_lossy();
            let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap() else {
                continue;
            };

            let decoded = group::decode(&blob, &gt)
                .unwrap_or_else(|e| panic!("{label}: group::decode ({gt}): {e}"));
            assert_eq!(
                group::encode(&decoded),
                blob,
                "{label}: group RawData round-trip differed for {gt}"
            );

            if let group::GroupVariant::Guild(g) = &decoded.data {
                if !g.base_ids.is_empty() {
                    populated_base_ids += 1;
                }
                if let group::GuildTail::PostUpdate(t) = &g.tail
                    && !t.role_permissions.is_empty()
                {
                    populated_role_perms += 1;
                }
            }
        }

        // Characters.
        let map = palsave::world::open_map(gvas, "CharacterSaveParameterMap").unwrap();
        let hpg = map.cursor.has_property_guid();
        for entry in &map.entries {
            let Some(rd) = find(&entry.fields, "RawData") else {
                continue;
            };
            let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap() else {
                continue;
            };
            let decoded = character::decode(&blob, hpg)
                .unwrap_or_else(|e| panic!("{label}: character::decode: {e}"));
            assert_eq!(
                character::encode(&blob, &decoded),
                blob,
                "{label}: character RawData round-trip differed"
            );
        }

        // Item containers and their slots.
        let map = palsave::world::open_map(gvas, "ItemContainerSaveData").unwrap();
        let mut slots_checked = 0usize;
        for entry in &map.entries {
            if let Some(rd) = find(&entry.fields, "RawData")
                && let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap()
            {
                let decoded = item_container::decode_container(&blob)
                    .unwrap_or_else(|e| panic!("{label}: decode_container: {e}"));
                assert_eq!(item_container::encode_container(&decoded), blob);
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
                let Some(rd) = find(fields, "RawData") else {
                    continue;
                };
                let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap() else {
                    continue;
                };
                let decoded = item_container::decode_slot(&blob)
                    .unwrap_or_else(|e| panic!("{label}: decode_slot: {e}"));
                assert_eq!(item_container::encode_slot(&decoded), blob);
                slots_checked += 1;
            }
        }

        // Pal containers (box, party, base camps) and their slots.
        let map = palsave::world::open_map(gvas, "CharacterContainerSaveData").unwrap();
        let mut pal_slots_checked = 0usize;
        for entry in &map.entries {
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
                let Some(rd) = find(fields, "RawData") else {
                    continue;
                };
                let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap() else {
                    continue;
                };
                // The decoder tolerates a longer tail so a future format bump doesn't
                // blank a Pal box. Pin the width here instead, where a change is a
                // signal rather than a user-visible failure.
                assert_eq!(
                    blob.len(),
                    38,
                    "{label}: Pal container slot blob is not 38 bytes — the layout moved"
                );
                let decoded = character_container::decode_slot(&blob)
                    .unwrap_or_else(|e| panic!("{label}: decode_slot: {e}"));
                assert_eq!(character_container::encode_slot(&decoded), blob);
                pal_slots_checked += 1;
            }
        }

        // Per-instance item state.
        let array = palsave::world::open_array(gvas, "DynamicItemSaveData").unwrap();
        let mut dynamic_items_checked = 0usize;
        for fields in &array.elements {
            let Some(rd) = find(fields, "RawData") else {
                continue;
            };
            let Value::Bytes(blob) = array.cursor.materialize(rd).unwrap() else {
                continue;
            };
            let decoded = dynamic_item::decode(&blob)
                .unwrap_or_else(|e| panic!("{label}: dynamic_item::decode: {e}"));
            assert_eq!(
                dynamic_item::encode(&decoded),
                blob,
                "{label}: DynamicItem RawData round-trip differed"
            );
            dynamic_items_checked += 1;
        }

        eprintln!(
            "{label}: ok — {dynamic_items_checked} dynamic items, {slots_checked} item slots, \
             {pal_slots_checked} pal slots, \
             {populated_base_ids} guild(s) with non-empty base_ids, {populated_role_perms} \
             with role_permissions"
        );
    }
}

/// Every `Level.sav` paired with the player saves sitting next to it.
///
/// `player_fixture()` returns one file from one world, which was fine while there was
/// only one. The two-file join is exactly the kind of thing that can pass on the world
/// it was written against and fail on the next, so the join tests below run over every
/// world and every player in it.
fn worlds_with_players() -> Vec<(PathBuf, Vec<PathBuf>)> {
    level_fixtures()
        .into_iter()
        .map(|level| {
            let dir = level.parent().unwrap().join("Players");
            let mut players: Vec<PathBuf> = std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "sav"))
                // `<uid>_dps.sav` is PalDimensionPalStorage, not a player file.
                .filter(|p| {
                    !p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().ends_with("_dps"))
                })
                .collect();
            players.sort();
            (level, players)
        })
        .collect()
}

/// The Pal-box join, over every world and every player.
///
/// Structural checks only — that the ids resolve and the shapes are sane. Whether the
/// *right* Pal landed in each slot is what the next test settles.
#[test]
fn pal_storage_resolves_from_the_player_save() {
    use palsave::inventory;

    let worlds = worlds_with_players();
    if worlds.iter().all(|(_, p)| p.is_empty()) {
        eprintln!("no player fixtures found, skipping");
        return;
    }

    for (level_path, players) in worlds {
        let level = palsave::container::decode(&std::fs::read(&level_path).unwrap()).unwrap();
        let label = level_path.display();

        for player_path in players {
            let player = palsave::container::decode(&std::fs::read(&player_path).unwrap()).unwrap();
            let uid = inventory::player_uid(&player.gvas).unwrap();

            let storage = inventory::player_pal_storage(&level.gvas, &player.gvas).unwrap();
            assert_eq!(storage.player_uid, uid);
            assert!(
                !storage.containers.is_empty(),
                "{label}/{uid}: no Pal containers — the SaveData lookup found nothing"
            );

            for container in &storage.containers {
                assert!(
                    !container.missing,
                    "{label}/{uid}: {:?} container {} named by the player save has no entry \
                     in Level.sav",
                    container.kind, container.id
                );
                assert!(
                    container.slots.len() <= container.slot_count as usize,
                    "{label}/{uid}: {:?} holds {} Pals in {} slots",
                    container.kind,
                    container.slots.len(),
                    container.slot_count
                );
                for (position, slot) in container.slots.iter().enumerate() {
                    assert_eq!(slot.slot_index, position as i32);
                    // A guid that decoded out of the wrong 16 bytes would still be a
                    // well-formed hex string, so the real check is that it names a Pal
                    // the world actually contains.
                    assert!(
                        slot.pal.is_some(),
                        "{label}/{uid}: {:?} slot {position} holds instance {} which is \
                         in no CharacterSaveParameterMap entry — the instance_id field \
                         is being read from the wrong offset",
                        container.kind,
                        slot.instance_id
                    );
                }
            }

            let total: usize = storage.containers.iter().map(|c| c.slots.len()).sum();
            assert!(total > 0, "{label}/{uid}: every Pal container is empty");
            eprintln!(
                "{label}/{uid}: {}",
                storage
                    .containers
                    .iter()
                    .map(|c| format!("{:?} {}/{}", c.kind, c.slots.len(), c.slot_count))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
}

/// The load-bearing one: two independent paths to "which Pals belong to this player"
/// must agree.
///
/// `pals_of` reads `OwnerPlayerUId` from inside each Pal's own RawData in
/// `CharacterSaveParameterMap`. `player_pal_storage` reads container ids from the
/// player's save and slot contents from `CharacterContainerSaveData`. Nothing is shared
/// between the two but the world itself, so a wrong offset in the container slot
/// decoder shows up here as a set mismatch rather than as plausible-looking output.
///
/// This mirrors `pals_of_agrees_with_independent_sources`, which is the pattern that
/// has caught the most real bugs in this crate.
#[test]
fn pal_storage_agrees_with_ownership() {
    use palsave::{characters, inventory};
    use std::collections::BTreeSet;

    let worlds = worlds_with_players();
    if worlds.iter().all(|(_, p)| p.is_empty()) {
        eprintln!("no player fixtures found, skipping");
        return;
    }

    for (level_path, players) in worlds {
        let level = palsave::container::decode(&std::fs::read(&level_path).unwrap()).unwrap();
        let label = level_path.display();

        for player_path in players {
            let player = palsave::container::decode(&std::fs::read(&player_path).unwrap()).unwrap();
            let uid = inventory::player_uid(&player.gvas).unwrap();

            let storage = inventory::player_pal_storage(&level.gvas, &player.gvas).unwrap();
            let in_containers: BTreeSet<String> = storage
                .containers
                .iter()
                .flat_map(|c| c.slots.iter())
                .map(|s| s.instance_id.clone())
                .collect();

            let owned: BTreeSet<String> = characters::pals_of(&level.gvas, &uid)
                .unwrap()
                .into_iter()
                .map(|p| p.instance_id)
                .collect();

            // Every Pal in this player's party or box must name them as owner.
            let unowned: Vec<&String> = in_containers.difference(&owned).collect();
            assert!(
                unowned.is_empty(),
                "{label}/{uid}: {} Pal(s) sit in this player's containers but do not name \
                 them as owner: {unowned:?}",
                unowned.len()
            );

            // The converse is only a warning: a Pal can name an owner while living in a
            // base-camp container, which is not reachable from the player's save.
            let elsewhere = owned.difference(&in_containers).count();
            eprintln!(
                "{label}/{uid}: {} in party+box, {} owned, {elsewhere} owned but housed \
                 elsewhere",
                in_containers.len(),
                owned.len()
            );
        }
    }
}

/// `DynamicItemSaveData` payload shapes must be unambiguous on real data.
///
/// The blob carries no item-type tag, so `dynamic_item` identifies a payload by
/// requiring exactly one of its three shapes to consume the bytes to the byte. That is
/// only sound if real blobs don't fit two shapes at once — the lengths *can* collide in
/// principle (a `WithAmmo` with an 18-character ammo name is 42 bytes, and so is the
/// shortest `Egg`).
///
/// So: assert no blob in any world is ambiguous, and report how many fall through to
/// `Opaque`. A rising `Opaque` count on a future save is the signal that a fourth shape
/// exists, and it shows up as a number here rather than as a wrong durability on screen.
#[test]
fn dynamic_item_shapes_are_unambiguous() {
    use palsave::gvas::nav::find;
    use palsave::gvas::value::Value;
    use palsave::rawdata::dynamic_item::{self, DynamicItemPayload};

    let levels = level_fixtures();
    if levels.is_empty() {
        eprintln!("no Level.sav fixtures found, skipping");
        return;
    }

    for path in levels {
        let container = palsave::container::decode(&std::fs::read(&path).unwrap()).unwrap();
        let gvas = &container.gvas;
        let label = path.display();

        let array = palsave::world::open_array(gvas, "DynamicItemSaveData").unwrap();
        let (mut durability, mut with_ammo, mut egg, mut opaque) = (0, 0, 0, 0);
        let mut with_durability_value = 0usize;

        for fields in &array.elements {
            let Some(rd) = find(fields, "RawData") else {
                continue;
            };
            let Value::Bytes(blob) = array.cursor.materialize(rd).unwrap() else {
                continue;
            };
            let decoded = dynamic_item::decode(&blob).unwrap();
            let name = decoded.static_id.display_lossy();

            // Re-derive the payload region the same way `decode` does, so the
            // ambiguity check sees exactly the bytes the classifier saw.
            let mut pos = 32usize;
            let _ = palsave::gvas::primitives::read_fstring(&blob, &mut pos).unwrap();
            let matches = dynamic_item::matching_shape_count(&blob[pos..]);
            assert!(
                matches <= 1,
                "{label}: {name} payload fits {matches} shapes at once — the exact-fit \
                 classifier can no longer tell them apart"
            );

            match &decoded.payload {
                DynamicItemPayload::Durability { .. } => durability += 1,
                DynamicItemPayload::WithAmmo { .. } => with_ammo += 1,
                DynamicItemPayload::Egg { .. } => egg += 1,
                DynamicItemPayload::Opaque(_) => opaque += 1,
            }
            if decoded.durability().is_some_and(|d| d > 0.0) {
                with_durability_value += 1;
            }
        }

        eprintln!(
            "{label}: {durability} durability, {with_ammo} with-ammo, {egg} egg, \
             {opaque} opaque ({with_durability_value} with a non-zero durability)"
        );

        // If every blob went opaque the classifier is doing nothing and the round-trip
        // test above would still pass, since Opaque round-trips perfectly.
        assert!(
            opaque < array.elements.len(),
            "{label}: no dynamic item payload was recognized at all"
        );
    }
}

/// Every item slot's `DynamicId` either resolves or is the all-zero sentinel.
///
/// This is the check that separates "most items legitimately have no dynamic state"
/// from "the join is broken and everything returns None". Both look identical on
/// screen; only the third category — a non-zero id that resolves to nothing — tells
/// them apart, and there must be none of it.
#[test]
fn dynamic_item_ids_resolve_or_are_zero() {
    use palsave::gvas::nav::find;
    use palsave::gvas::value::Value;
    use palsave::rawdata::{dynamic_item, item_container};
    use std::collections::BTreeSet;

    let levels = level_fixtures();
    if levels.is_empty() {
        eprintln!("no Level.sav fixtures found, skipping");
        return;
    }

    for path in levels {
        let container = palsave::container::decode(&std::fs::read(&path).unwrap()).unwrap();
        let gvas = &container.gvas;
        let label = path.display();

        let array = palsave::world::open_array(gvas, "DynamicItemSaveData").unwrap();
        let known: BTreeSet<item_container::DynamicId> = array
            .elements
            .iter()
            .filter_map(|fields| {
                let rd = find(fields, "RawData")?;
                let Value::Bytes(blob) = array.cursor.materialize(rd).ok()? else {
                    return None;
                };
                dynamic_item::decode(&blob).ok().map(|d| d.id)
            })
            .collect();
        assert!(!known.is_empty(), "{label}: no dynamic items decoded");

        let map = palsave::world::open_map(gvas, "ItemContainerSaveData").unwrap();
        let (mut zero, mut resolved, mut dangling) = (0usize, 0usize, 0usize);

        for entry in &map.entries {
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
                let Some(rd) = find(fields, "RawData") else {
                    continue;
                };
                let Value::Bytes(blob) = map.cursor.materialize(rd).unwrap() else {
                    continue;
                };
                let Ok(decoded) = item_container::decode_slot(&blob) else {
                    continue;
                };
                if decoded.count <= 0 {
                    continue;
                }
                let id = &decoded.item.dynamic_id;
                if id.is_zero() {
                    zero += 1;
                } else if known.contains(id) {
                    resolved += 1;
                } else {
                    dangling += 1;
                }
            }
        }

        assert_eq!(
            dangling, 0,
            "{label}: {dangling} occupied slot(s) reference a non-zero DynamicId that no \
             DynamicItemSaveData row matches — the join key is wrong"
        );
        // Every dynamic item exists because some slot points at it, so a world with
        // dynamic items must have slots that resolve. Zero here would mean the join
        // silently produced nothing while every assertion above still passed.
        assert!(
            resolved > 0,
            "{label}: not one occupied slot resolved to a dynamic item, though {} exist",
            known.len()
        );
        eprintln!("{label}: {resolved} slots resolved, {zero} plain, {dangling} dangling");
    }
}

/// Inserting a map entry and removing it again must return the exact original bytes.
///
/// This is the gate for the capability migration is blocked on. An insert touches three
/// things — the entry bytes, the u32 entry count, and every enclosing `size` — and the
/// claim is that there is no fourth. A round-trip to byte-identity is what makes that
/// claim falsifiable: any missed fixup leaves a difference somewhere in the buffer, and
/// any *extra* write shows up the same way.
///
/// Run over `CharacterSaveParameterMap` because that is the map a migrated player and
/// their Pals actually land in, and it is the largest, so a size fixup that overflows or
/// lands on the wrong ancestor has the most room to show itself.
#[test]
fn inserting_then_removing_a_map_entry_restores_the_original_bytes() {
    use palsave::edit;
    use palsave::gvas::GvasFile;

    for level in level_fixtures() {
        let container = palsave::container::decode(&std::fs::read(&level).unwrap()).unwrap();
        let original = &container.gvas;
        let label = level.display();

        let path = "worldSaveData.CharacterSaveParameterMap";
        let file = GvasFile::parse(original).unwrap();
        let engine_major = file.header.engine_version_major;
        let has_property_guid = file.header.has_property_guid();

        let map = palsave::world::open_map(original, "CharacterSaveParameterMap").unwrap();
        let chain = [&map.world_entry, &map.map_entry];

        let before = edit::map_layout_entry_count(
            original,
            &map.map_entry,
            engine_major,
            has_property_guid,
            path,
        );

        // Copy entry 0 and append it. A duplicate key is a corrupt save the game would
        // dislike, but it is structurally identical to inserting a foreign entry, which
        // is what migration does — and it needs no second world to be present.
        let entry = edit::map_entry_bytes(
            original,
            &map.map_entry,
            0,
            engine_major,
            has_property_guid,
            path,
        )
        .unwrap();
        assert!(!entry.0.is_empty(), "{label}: entry 0 has no bytes");

        let inserted = edit::insert_map_entry(
            original,
            &chain,
            &entry,
            engine_major,
            has_property_guid,
            path,
        )
        .unwrap()
        .apply(original)
        .unwrap();

        // The strongest single check the format offers: parse must reproduce the buffer
        // exactly, which is only true when every size field agrees at every depth.
        edit::verify_reparses(&inserted).unwrap_or_else(|e| {
            panic!("{label}: buffer with an inserted entry did not re-parse: {e}")
        });
        assert_eq!(inserted.len(), original.len() + entry.0.len());

        // And the map really grew, rather than the bytes landing somewhere inert.
        let after_map = palsave::world::open_map(&inserted, "CharacterSaveParameterMap").unwrap();
        let after = edit::map_layout_entry_count(
            &inserted,
            &after_map.map_entry,
            engine_major,
            has_property_guid,
            path,
        );
        assert_eq!(
            after,
            before + 1,
            "{label}: entry count did not grow by one"
        );

        let appended = edit::map_entry_bytes(
            &inserted,
            &after_map.map_entry,
            after - 1,
            engine_major,
            has_property_guid,
            path,
        )
        .unwrap();
        assert_eq!(
            appended, entry,
            "{label}: the appended entry read back changed"
        );

        // Now take it out again.
        let after_chain = [&after_map.world_entry, &after_map.map_entry];
        let restored = edit::remove_map_entry(
            &inserted,
            &after_chain,
            after - 1,
            engine_major,
            has_property_guid,
            path,
        )
        .unwrap()
        .apply(&inserted)
        .unwrap();

        assert_eq!(
            restored.len(),
            original.len(),
            "{label}: removing the inserted entry did not restore the length"
        );
        assert!(
            restored == *original,
            "{label}: insert-then-remove is not the identity — a fixup is missing or extra"
        );

        eprintln!("{label}: {before} entries, +1 and back, byte-identical");
    }
}

/// No Palworld map records pending key removals.
///
/// `edit::insert_map_entry` refuses such a map rather than guess where the entry count
/// sits after them. That refusal is only a reasonable design if the case genuinely
/// doesn't arise — so check, across every map in every world, rather than assume.
#[test]
fn map_layouts_have_no_pending_key_removals() {
    use palsave::gvas::GvasFile;
    use palsave::gvas::value::map_layout;

    for level in level_fixtures() {
        let container = palsave::container::decode(&std::fs::read(&level).unwrap()).unwrap();
        let gvas = &container.gvas;
        let label = level.display();

        let file = GvasFile::parse(gvas).unwrap();
        let engine_major = file.header.engine_version_major;
        let has_property_guid = file.header.has_property_guid();

        let world_idx = file
            .properties
            .iter()
            .position(|p| p.name == "worldSaveData")
            .unwrap();
        let world = file.materialize(world_idx).unwrap();
        let children = world.as_properties().unwrap();

        let mut checked = 0usize;
        for child in children {
            if child.type_name != "MapProperty" {
                continue;
            }
            let path = format!("worldSaveData.{}", child.name);
            // Maps this crate can't fully walk are skipped, not failed — the claim
            // under test is about the ones the edit path can actually reach.
            let Ok(layout) = map_layout(gvas, child, engine_major, has_property_guid, &path) else {
                continue;
            };
            assert_eq!(
                layout.removed_count, 0,
                "{label}: {} records {} pending key removal(s); \
                 edit::insert_map_entry's refusal would now be a live limitation",
                child.name, layout.removed_count
            );
            checked += 1;
        }
        assert!(checked > 0, "{label}: no maps were walkable at all");
        eprintln!("{label}: {checked} maps, none with pending key removals");
    }
}

/// Guild members and players must agree on what a uid looks like.
///
/// They did not. `guilds.rs` carried its own copy of `guid_to_hex` that was never
/// updated when `nav::guid_to_hex` moved to Unreal's display convention, so the same
/// player appeared as `…01000000` on the Guilds screen and `…00000001` on the Players
/// screen. Every existing test passed: they compared *counts*, never the ids
/// themselves, and guild ids are opaque handles that stayed self-consistent.
///
/// This is the check that fails when the two formatters drift apart again.
#[test]
fn guild_member_uids_agree_with_players_and_save_filenames() {
    use palsave::{characters, guilds};

    for level in level_fixtures() {
        let container = palsave::container::decode(&std::fs::read(&level).unwrap()).unwrap();
        let gvas = &container.gvas;

        let player_uids: Vec<String> = characters::list_players(gvas)
            .unwrap()
            .into_iter()
            .map(|p| p.uid)
            .collect();
        if player_uids.is_empty() {
            continue;
        }

        for summary in guilds::list(gvas).unwrap() {
            if summary.group_type != palsave::rawdata::group::GUILD {
                continue;
            }
            let detail = guilds::detail(gvas, &summary.id).unwrap();

            for member in &detail.members {
                assert!(
                    player_uids.contains(&member.player_uid),
                    "{}: guild member {} is not among the players {player_uids:?} — the two \
                     layers disagree about uid formatting",
                    level.display(),
                    member.player_uid
                );
            }
            if let Some(admin) = &detail.admin_player_uid {
                assert!(
                    player_uids.contains(admin),
                    "{}: guild admin {admin} is not among the players {player_uids:?}",
                    level.display()
                );
            }
        }

        // And the uid really is the player's own save filename.
        let players_dir = level.parent().unwrap().join("Players");
        if let Ok(entries) = std::fs::read_dir(&players_dir) {
            let stems: Vec<String> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "sav"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
                // `<uid>_dps.sav` is the separate Pal-storage save, not a player file.
                .filter(|s| !s.ends_with("_dps"))
                .collect();
            for uid in &player_uids {
                assert!(
                    stems.contains(&uid.to_lowercase()),
                    "{}: player uid {uid} matches no Players/*.sav filename {stems:?}",
                    level.display()
                );
            }
        }
    }
}
