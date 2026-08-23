//! Synthetic Palworld saves, built from this crate's own writers.
//!
//! Real saves are gitignored — they carry SteamIDs and player names — so anything
//! that needs a `.sav` in CI (the wasm boundary tests, the browser end-to-end suite)
//! has to construct one. These builders emit **structurally real** files: a proper
//! GVAS header, a `worldSaveData` holding a guild, a player, a Pal, an item container,
//! two Pal containers and a dynamic-item row, plus a matching player save. Built
//! inner-to-outer so every `size` field is exact, which is what makes them useful as
//! parser input rather than just bytes.
//!
//! The contents are chosen so every join in the crate has something to resolve: the
//! Pal is in the Pal box *and* names the player as owner, and the item slot's
//! `DynamicId` is non-zero so the durability lookup is exercised rather than
//! short-circuited by the "no per-instance state" sentinel. A fixture where the joins
//! trivially return nothing would let a broken decoder pass.
//!
//! Behind the `synthetic` feature so none of it ships in the released wasm. The
//! feature is enabled for `palsave-wasm`'s dev-dependency, so `cargo test` and
//! `clippy --all-targets` see it while `cargo build --release` does not.

use crate::container::{self, Algorithm, Container, Passes};
use crate::gvas::header::{GVAS_MAGIC, Header};
use crate::gvas::primitives::{FString, write_fstring, write_i32_le, write_u32_le};
use crate::gvas::property::{PropertyTag, TagExtra, none_terminator, write_property_tag};
use crate::rawdata::{character_container, dynamic_item, group, item_container};

/// Every identity a synthetic world is built around, so a second, *different* world can
/// be generated from the same builders.
///
/// A migration is a two-world question and cannot be tested against one world, however
/// carefully built. Parameterising the ids is what makes the second world possible.
#[derive(Debug, Clone, Copy)]
pub struct WorldIds {
    pub guild_id: [u8; 16],
    pub player_uid: [u8; 16],
    pub player_instance_id: [u8; 16],
    pub pal_instance_id: [u8; 16],
    pub container_id: [u8; 16],
    pub pal_storage_container_id: [u8; 16],
    pub pal_party_container_id: [u8; 16],
    pub dynamic_item_local_id: [u8; 16],
}

/// The default world. All ids obviously-synthetic: test data, never lifted from a real
/// save.
pub const WORLD_A: WorldIds = WorldIds {
    guild_id: [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ],
    // Little-endian 1 in the final u32 group, so this renders as
    // "00000000000000000000000000000001" — Palworld's own player-file naming.
    player_uid: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0, 0, 0],
    player_instance_id: [0x77; 16],
    pal_instance_id: [0x55; 16],
    container_id: [0x42; 16],
    pal_storage_container_id: [0x43; 16],
    pal_party_container_id: [0x44; 16],
    dynamic_item_local_id: [0x66; 16],
};

/// A second, unrelated world — except for one deliberate overlap.
///
/// `pal_instance_id` is **the same as [`WORLD_A`]'s**, reproducing the real behaviour
/// found in the fixture corpus: Pal instance ids are not globally unique, so two
/// unrelated worlds can contain the same one. Without that overlap the collision path
/// in `migrate` would have nothing to detect and its tests would pass vacuously.
///
/// The player uid deliberately *differs*, so a migration between these two isolates the
/// Pal collision instead of drowning it in a player collision as well.
pub const WORLD_B: WorldIds = WorldIds {
    guild_id: [0xb1; 16],
    player_uid: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0, 0, 0],
    player_instance_id: [0xb7; 16],
    // Shared with WORLD_A on purpose. See above.
    pal_instance_id: [0x55; 16],
    container_id: [0xb2; 16],
    pal_storage_container_id: [0xb3; 16],
    pal_party_container_id: [0xb4; 16],
    dynamic_item_local_id: [0xb6; 16],
};

/// Kept for tests written before worlds were parameterised.
pub const GUILD_ID: [u8; 16] = WORLD_A.guild_id;

pub fn ascii(s: &str) -> FString {
    FString::Ascii {
        content: s.as_bytes().to_vec(),
        trailing: vec![0],
    }
}

/// One guild's RawData blob, in the current (`PostUpdate` tail) shape.
fn guild_blob(ids: &WorldIds) -> Vec<u8> {
    group::encode(&group::GroupData {
        group_id: ids.guild_id,
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
fn group_entry_value(ids: &WorldIds) -> Vec<u8> {
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

    let blob = guild_blob(ids);
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

pub const PLAYER_UID: [u8; 16] = WORLD_A.player_uid;
pub const PLAYER_INSTANCE_ID: [u8; 16] = WORLD_A.player_instance_id;
/// The container the synthetic player's InventoryInfo points at, and the key of the
/// one entry in the synthetic level's ItemContainerSaveData. The join under test.
pub const CONTAINER_ID: [u8; 16] = [0x42; 16];
/// The one Pal in the synthetic world. Sits in the Pal box *and* names the player as
/// owner, so both routes to "whose Pal is this" resolve to it.
pub const PAL_INSTANCE_ID: [u8; 16] = [0x55; 16];
/// Keys of the two `CharacterContainerSaveData` entries the player's save points at.
pub const PAL_STORAGE_CONTAINER_ID: [u8; 16] = [0x43; 16];
pub const PAL_PARTY_CONTAINER_ID: [u8; 16] = [0x44; 16];
/// The `DynamicItemSaveData` row the synthetic item slot references. Non-zero on
/// purpose: an all-zero id is the "no dynamic state" sentinel and would exercise
/// nothing.
pub const DYNAMIC_ITEM_LOCAL_ID: [u8; 16] = [0x66; 16];

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
fn player_raw_data(ids: &WorldIds) -> Vec<u8> {
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
    blob.extend_from_slice(&ids.guild_id); // group_id
    blob.extend_from_slice(&[0, 0, 0, 0]); // trailing_bytes
    blob
}

/// A `SaveParameter` for a Pal: no `IsPlayer` flag, an owner, a species and IVs. The
/// owner is what `characters::pals_of` reads, so a Pal built this way is reachable
/// both through ownership and through the Pal box — the two paths
/// `pal_storage_agrees_with_ownership` compares.
fn pal_save_parameter(ids: &WorldIds) -> Vec<u8> {
    let mut out = Vec::new();

    let character_id = {
        let mut v = Vec::new();
        write_fstring(&mut v, &ascii("Lamball"));
        v
    };
    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("CharacterID"),
            type_name: ascii("NameProperty"),
            size: character_id.len() as u32,
            index: 0,
            extra: TagExtra::None,
            guid: None,
        },
        true,
    );
    out.extend_from_slice(&character_id);

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
    out.push(12);

    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("Talent_HP"),
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
    out.push(70);

    write_property_tag(
        &mut out,
        &PropertyTag {
            name: ascii("OwnerPlayerUId"),
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
    out.extend_from_slice(&ids.player_uid);

    write_fstring(&mut out, &none_terminator());
    out
}

/// Wraps a `SaveParameter` list into the `PalCharacterData` RawData blob shape.
fn character_raw_data(ids: &WorldIds, save_parameter: Vec<u8>) -> Vec<u8> {
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
    blob.extend_from_slice(&ids.guild_id); // group_id
    blob.extend_from_slice(&[0, 0, 0, 0]); // trailing_bytes
    blob
}

/// One `CharacterSaveParameterMap` entry body: the key property list, then the value
/// holding the RawData blob.
fn character_map_entry(player_uid: [u8; 16], instance_id: [u8; 16], blob: Vec<u8>) -> Vec<u8> {
    let mut v = Vec::new();

    // Key: a property list (PlayerUId + InstanceId), per gvas::hints.
    let mut key = Vec::new();
    for (name, guid) in [("PlayerUId", player_uid), ("InstanceId", instance_id)] {
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

/// `CharacterSaveParameterMap`: the player, plus one Pal that sits in their Pal box.
///
/// A Pal is not optional decoration here. Without one the Pal-box join has nothing to
/// resolve to, and a test asserting "the box is readable" would pass against a decoder
/// that returns nothing.
fn character_map_value(ids: &WorldIds) -> Vec<u8> {
    let mut v = Vec::new();
    write_u32_le(&mut v, 0); // keys-to-remove count
    write_u32_le(&mut v, 2); // entry count

    v.extend_from_slice(&character_map_entry(
        ids.player_uid,
        ids.player_instance_id,
        player_raw_data(ids),
    ));
    // A Pal's map key carries a zero PlayerUId — ownership lives in the RawData.
    v.extend_from_slice(&character_map_entry(
        [0u8; 16],
        ids.pal_instance_id,
        character_raw_data(ids, pal_save_parameter(ids)),
    ));

    v
}

/// One `ItemContainerSaveData` entry holding **two** occupied slots.
///
/// Two, not one, and the difference is load-bearing:
///
/// - Slot 0's `DynamicId` is non-zero and matches [`dynamic_item_array_value`], so the
///   durability join has something to resolve.
/// - Slot 1's is the all-zero sentinel — an ordinary stack with no per-instance state,
///   the *common* case in a real save.
///
/// Only having the first meant every field the UI reads was always populated, so the
/// "this item has no durability" render path was never exercised by any test. That gap
/// shipped a crash: `Option::None` crosses the wasm boundary as `undefined`, a
/// `!== null` guard let it through, and the Pals & items tab hung on "Loading…" for
/// every real save. A fixture where everything is present cannot catch a
/// missing-field bug.
fn item_container_map_value(ids: &WorldIds) -> Vec<u8> {
    let mut v = Vec::new();
    write_u32_le(&mut v, 0); // keys-to-remove count
    write_u32_le(&mut v, 1); // entry count

    v.extend_from_slice(&container_key(ids.container_id));

    let with_state = item_container::encode_slot(&item_container::ItemContainerSlot {
        slot_index: 0,
        count: 5,
        item: item_container::ItemId {
            static_id: ascii("ClothArmor"),
            dynamic_id: item_container::DynamicId {
                created_world_id: [0u8; 16],
                local_id_in_created_world: ids.dynamic_item_local_id,
            },
        },
        trailing_bytes: Vec::new(),
    });
    let plain = item_container::encode_slot(&item_container::ItemContainerSlot {
        slot_index: 1,
        count: 63,
        item: item_container::ItemId {
            static_id: ascii("Wood"),
            // All-zero: one plank is like any other, so there is no row to join to.
            dynamic_id: item_container::DynamicId {
                created_world_id: [0u8; 16],
                local_id_in_created_world: [0u8; 16],
            },
        },
        trailing_bytes: Vec::new(),
    });
    v.extend_from_slice(&container_value(
        &[raw_data_property(&with_state), raw_data_property(&plain)],
        "PalItemSlotSaveData",
        42,
    ));

    v
}

/// A `{ID: Guid}` map key, the shape both container maps use.
fn container_key(id: [u8; 16]) -> Vec<u8> {
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
    key.extend_from_slice(&id);
    write_fstring(&mut key, &none_terminator());
    key
}

/// Wraps a rawdata blob as a `RawData` ArrayProperty inside a property list.
fn raw_data_property(blob: &[u8]) -> Vec<u8> {
    let raw_value = {
        let mut r = Vec::new();
        write_u32_le(&mut r, blob.len() as u32);
        r.extend_from_slice(blob);
        r
    };
    let mut body = Vec::new();
    write_property_tag(
        &mut body,
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
    body.extend_from_slice(&raw_value);
    write_fstring(&mut body, &none_terminator());
    body
}

/// A container value: `Slots` plus `SlotNum`.
fn container_value(slot_bodies: &[Vec<u8>], struct_type: &str, slot_num: i32) -> Vec<u8> {
    // The nested element tag's `size` is the total of *all* element bodies, not one of
    // them — measured on real saves by
    // `array_inner_tag_size_covers_all_element_bodies`. With a single-slot fixture the
    // two were indistinguishable, so writing it correctly only started to matter once
    // a container held more than one thing.
    let bodies_len: usize = slot_bodies.iter().map(|b| b.len()).sum();

    // Slots: ArrayProperty<StructProperty>. The element tag is written once, before
    // the bodies — and is present even for a zero-length array (see ADR-003.md).
    let slots_value = {
        let mut a = Vec::new();
        write_u32_le(&mut a, slot_bodies.len() as u32); // element count
        write_property_tag(
            &mut a,
            &PropertyTag {
                name: ascii("Slots"),
                type_name: ascii("StructProperty"),
                size: bodies_len as u32,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii(struct_type),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );
        for body in slot_bodies {
            a.extend_from_slice(body);
        }
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
    write_i32_le(&mut value, slot_num);
    write_fstring(&mut value, &none_terminator());
    value
}

/// `CharacterContainerSaveData`: the player's Pal box (holding the one Pal) and their
/// party (empty capacity, so the "container resolves but is empty" path is exercised
/// too).
fn pal_container_map_value(ids: &WorldIds) -> Vec<u8> {
    let mut v = Vec::new();
    write_u32_le(&mut v, 0); // keys-to-remove count
    write_u32_le(&mut v, 2); // entry count

    let occupied = character_container::encode_slot(&character_container::PalContainerSlot {
        leading_bytes: ids.player_uid,
        instance_id: ids.pal_instance_id,
        trailing_bytes: vec![0; 6],
    });
    v.extend_from_slice(&container_key(ids.pal_storage_container_id));
    v.extend_from_slice(&container_value(
        &[raw_data_property(&occupied)],
        "PalContainerCharacterSlotSaveData",
        960,
    ));

    // The party slot points at the same Pal. That is not how a real save looks, but it
    // keeps the fixture to one Pal while still proving both containers resolve — and a
    // test that cares would compare instance ids, which are identical either way.
    v.extend_from_slice(&container_key(ids.pal_party_container_id));
    v.extend_from_slice(&container_value(
        &[raw_data_property(&occupied)],
        "PalContainerCharacterSlotSaveData",
        5,
    ));

    v
}

/// `DynamicItemSaveData`: one row, matching the id on the synthetic item slot.
fn dynamic_item_array_value(ids: &WorldIds) -> Vec<u8> {
    let blob = dynamic_item::encode(&dynamic_item::DynamicItem {
        id: item_container::DynamicId {
            created_world_id: [0u8; 16],
            local_id_in_created_world: ids.dynamic_item_local_id,
        },
        static_id: ascii("ClothArmor"),
        payload: dynamic_item::DynamicItemPayload::Durability {
            unknown_0: 0,
            durability: 150.0,
            remaining_bullets: 0,
        },
    });
    let element = raw_data_property(&blob);

    let mut v = Vec::new();
    write_u32_le(&mut v, 1); // element count
    write_property_tag(
        &mut v,
        &PropertyTag {
            name: ascii("DynamicItemSaveData"),
            type_name: ascii("StructProperty"),
            size: element.len() as u32,
            index: 0,
            extra: TagExtra::Struct {
                struct_type: ascii("PalDynamicItemSaveData"),
                guid: [0u8; 16],
            },
            guid: None,
        },
        true,
    );
    v.extend_from_slice(&element);
    v
}

/// A `Players/<uid>.sav`: the save class that identifies it, plus the `SaveData`
/// holding the player's uid and the container ids that make an inventory resolvable.
pub fn synthetic_player_sav() -> Vec<u8> {
    synthetic_player_sav_for(&WORLD_A)
}

/// A `Players/<uid>.sav` for a specific world's identities.
pub fn synthetic_player_sav_for(ids: &WorldIds) -> Vec<u8> {
    // A `<Name>ContainerId` is a struct wrapping a single `ID` guid — the same shape
    // whether it names an item container or a Pal container.
    let container_id_struct = |id: [u8; 16]| {
        let mut value = Vec::new();
        write_property_tag(
            &mut value,
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
        value.extend_from_slice(&id);
        write_fstring(&mut value, &none_terminator());
        value
    };
    let write_container_id = |out: &mut Vec<u8>, name: &str, id: [u8; 16]| {
        let body = container_id_struct(id);
        write_property_tag(
            out,
            &PropertyTag {
                name: ascii(name),
                type_name: ascii("StructProperty"),
                size: body.len() as u32,
                index: 0,
                extra: TagExtra::Struct {
                    struct_type: ascii("PalContainerId"),
                    guid: [0u8; 16],
                },
                guid: None,
            },
            true,
        );
        out.extend_from_slice(&body);
    };

    let mut inventory_info = Vec::new();
    write_container_id(&mut inventory_info, "CommonContainerId", ids.container_id);
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
    save_data.extend_from_slice(&ids.player_uid);
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

    // Pal containers sit at SaveData top level, not under InventoryInfo — the
    // distinction `inventory::PAL_CONTAINER_KINDS` exists for.
    write_container_id(
        &mut save_data,
        "OtomoCharacterContainerId",
        ids.pal_party_container_id,
    );
    write_container_id(
        &mut save_data,
        "PalStorageContainerId",
        ids.pal_storage_container_id,
    );

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
    synthetic_sav_for(&WORLD_A)
}

/// A `Level.sav` built around a specific world's identities.
pub fn synthetic_sav_for(ids: &WorldIds) -> Vec<u8> {
    let map_value = {
        let mut v = Vec::new();
        write_u32_le(&mut v, 0); // keys-to-remove count
        write_u32_le(&mut v, 1); // entry count
        v.extend_from_slice(&ids.guild_id); // key: bare Guid (per gvas::hints)
        v.extend_from_slice(&group_entry_value(ids));
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

        let character_value = character_map_value(ids);
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

        let item_value = item_container_map_value(ids);
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

        let pal_container_value = pal_container_map_value(ids);
        write_property_tag(
            &mut v,
            &PropertyTag {
                name: ascii("CharacterContainerSaveData"),
                type_name: ascii("MapProperty"),
                size: pal_container_value.len() as u32,
                index: 0,
                extra: TagExtra::Map {
                    key_type: ascii("StructProperty"),
                    value_type: ascii("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        v.extend_from_slice(&pal_container_value);

        let dynamic_value = dynamic_item_array_value(ids);
        write_property_tag(
            &mut v,
            &PropertyTag {
                name: ascii("DynamicItemSaveData"),
                type_name: ascii("ArrayProperty"),
                size: dynamic_value.len() as u32,
                index: 0,
                extra: TagExtra::Array {
                    inner_type: ascii("StructProperty"),
                },
                guid: None,
            },
            true,
        );
        v.extend_from_slice(&dynamic_value);

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
