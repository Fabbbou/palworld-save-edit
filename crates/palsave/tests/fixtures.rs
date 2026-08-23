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
