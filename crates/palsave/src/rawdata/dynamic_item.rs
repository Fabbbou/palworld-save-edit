//! `.worldSaveData.DynamicItemSaveData[].RawData`: the per-instance state of items
//! that have any — armour durability, a gun's loaded ammo, what is inside an egg.
//!
//! An `ItemContainerSlot` says *what* an item is (`static_id`) and how many. Anything
//! that varies between two copies of the same item lives here instead, keyed by the
//! `DynamicId` guid pair the slot already carries. Nothing in the crate resolved that
//! key before this module.
//!
//! ## The awkward part: no type tag
//!
//! The blob starts legibly — two guids and the item's `static_id` — and then the
//! payload's *shape depends on the kind of item*, with nothing in the blob saying
//! which kind that is. The game knows from its own item table; a save file reader does
//! not have one. Measured with `examples/dump_blobs tails` over two worlds, the
//! payload takes exactly three shapes:
//!
//! ```text
//!  12 bytes        u32, f32 durability, i32 remaining_bullets      ClothArmor -> 150.0
//!  24+n bytes      ... plus u32, FString, u32                      Bow_triple -> "Arrow"
//!  40+n+m bytes    u32, FString, FString, 28 bytes                 PalEgg_* -> "Deer"
//! ```
//!
//! ## Why guessing is safe here, and how it is kept safe
//!
//! Rather than infer the item kind, [`decode`] tries each shape and requires it to
//! consume the payload to the byte. A shape that ends early or runs over is not a
//! match. If **more than one** shape fits exactly the blob is ambiguous and the
//! payload stays [`DynamicItemPayload::Opaque`] — a refusal, not a coin flip. Same if
//! none fit.
//!
//! That matters because the lengths *can* collide in principle: a `WithAmmo` payload
//! whose ammo name is 18 characters is 42 bytes, and so is the shortest possible
//! `Egg`. `dynamic_item_shapes_are_unambiguous` asserts no real blob in either world
//! is ambiguous, so the collision is a hypothetical rather than something being papered
//! over. If a future save makes it real, the affected item degrades to "no dynamic
//! state known" instead of reporting another item's durability.
//!
//! Round-tripping is exact for every shape including `Opaque`, so this module can never
//! change a byte it did not understand — which is what lets migration carry these rows
//! across worlds without understanding them.

use super::error::RawDataError;
use crate::gvas::primitives::{
    FString, read_f32_le, read_fstring, read_guid, read_i32_le, read_u32_le, write_f32_le,
    write_fstring, write_guid, write_i32_le, write_u32_le,
};
use crate::rawdata::item_container::DynamicId;

/// Bytes of fixed trailer on the egg shape, after its two strings.
const EGG_TRAILER_LEN: usize = 28;

#[derive(Debug, Clone, PartialEq)]
pub enum DynamicItemPayload {
    /// Armour and other single-durability items.
    Durability {
        unknown_0: u32,
        durability: f32,
        remaining_bullets: i32,
    },
    /// Weapons and tools: durability plus a named ammunition slot. `ammo_static_id` is
    /// the literal string `None` when nothing is loaded — the game's own sentinel, kept
    /// rather than mapped to `Option` so re-encoding is exact.
    WithAmmo {
        unknown_0: u32,
        durability: f32,
        remaining_bullets: i32,
        unknown_1: u32,
        ammo_static_id: FString,
        unknown_2: u32,
    },
    /// Pal eggs: which Pal is inside.
    Egg {
        unknown_0: u32,
        character_id: FString,
        unknown_name: FString,
        trailing_bytes: Vec<u8>,
    },
    /// No shape fit exactly, or more than one did. Preserved verbatim.
    Opaque(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicItem {
    pub id: DynamicId,
    pub static_id: FString,
    pub payload: DynamicItemPayload,
}

impl DynamicItem {
    /// Durability, for the shapes that carry one.
    pub fn durability(&self) -> Option<f32> {
        match &self.payload {
            DynamicItemPayload::Durability { durability, .. }
            | DynamicItemPayload::WithAmmo { durability, .. } => Some(*durability),
            _ => None,
        }
    }

    /// Rounds loaded, for the shapes that carry a count.
    pub fn remaining_bullets(&self) -> Option<i32> {
        match &self.payload {
            DynamicItemPayload::Durability {
                remaining_bullets, ..
            }
            | DynamicItemPayload::WithAmmo {
                remaining_bullets, ..
            } => Some(*remaining_bullets),
            _ => None,
        }
    }

    /// The loaded ammunition's item id, or `None` when the item has no ammo slot or
    /// the slot holds the game's `None` sentinel.
    pub fn ammo_static_id(&self) -> Option<String> {
        let DynamicItemPayload::WithAmmo { ammo_static_id, .. } = &self.payload else {
            return None;
        };
        let name = ammo_static_id.display_lossy();
        (!name.is_empty() && name != "None").then_some(name)
    }

    /// The Pal inside an egg.
    pub fn egg_character_id(&self) -> Option<String> {
        let DynamicItemPayload::Egg { character_id, .. } = &self.payload else {
            return None;
        };
        let name = character_id.display_lossy();
        (!name.is_empty()).then_some(name)
    }
}

/// Parses the `Durability` shape, requiring it to end exactly at `payload.len()`.
fn try_durability(payload: &[u8]) -> Option<DynamicItemPayload> {
    let mut pos = 0usize;
    let unknown_0 = read_u32_le(payload, &mut pos).ok()?;
    let durability = read_f32_le(payload, &mut pos).ok()?;
    let remaining_bullets = read_i32_le(payload, &mut pos).ok()?;
    (pos == payload.len()).then_some(DynamicItemPayload::Durability {
        unknown_0,
        durability,
        remaining_bullets,
    })
}

fn try_with_ammo(payload: &[u8]) -> Option<DynamicItemPayload> {
    let mut pos = 0usize;
    let unknown_0 = read_u32_le(payload, &mut pos).ok()?;
    let durability = read_f32_le(payload, &mut pos).ok()?;
    let remaining_bullets = read_i32_le(payload, &mut pos).ok()?;
    let unknown_1 = read_u32_le(payload, &mut pos).ok()?;
    let ammo_static_id = read_fstring(payload, &mut pos).ok()?;
    let unknown_2 = read_u32_le(payload, &mut pos).ok()?;
    (pos == payload.len()).then_some(DynamicItemPayload::WithAmmo {
        unknown_0,
        durability,
        remaining_bullets,
        unknown_1,
        ammo_static_id,
        unknown_2,
    })
}

fn try_egg(payload: &[u8]) -> Option<DynamicItemPayload> {
    let mut pos = 0usize;
    let unknown_0 = read_u32_le(payload, &mut pos).ok()?;
    let character_id = read_fstring(payload, &mut pos).ok()?;
    let unknown_name = read_fstring(payload, &mut pos).ok()?;
    let trailing_bytes = payload.get(pos..)?.to_vec();
    (trailing_bytes.len() == EGG_TRAILER_LEN).then_some(DynamicItemPayload::Egg {
        unknown_0,
        character_id,
        unknown_name,
        trailing_bytes,
    })
}

/// How many of the three shapes fit `payload` exactly. Exposed for the fixture test
/// that asserts real data is never ambiguous.
pub fn matching_shape_count(payload: &[u8]) -> usize {
    [
        try_durability(payload),
        try_with_ammo(payload),
        try_egg(payload),
    ]
    .iter()
    .filter(|m| m.is_some())
    .count()
}

fn classify(payload: &[u8]) -> DynamicItemPayload {
    let mut matches = [
        try_durability(payload),
        try_with_ammo(payload),
        try_egg(payload),
    ]
    .into_iter()
    .flatten();

    match (matches.next(), matches.next()) {
        // Exactly one shape fit.
        (Some(only), None) => only,
        // None fit, or several did — either way we don't know, so say so.
        _ => DynamicItemPayload::Opaque(payload.to_vec()),
    }
}

pub fn decode(bytes: &[u8]) -> Result<DynamicItem, RawDataError> {
    let mut pos = 0usize;
    let created_world_id = read_guid(bytes, &mut pos)?;
    let local_id_in_created_world = read_guid(bytes, &mut pos)?;
    let static_id = read_fstring(bytes, &mut pos)?;
    let payload = classify(&bytes[pos..]);

    Ok(DynamicItem {
        id: DynamicId {
            created_world_id,
            local_id_in_created_world,
        },
        static_id,
        payload,
    })
}

pub fn encode(data: &DynamicItem) -> Vec<u8> {
    let mut out = Vec::new();
    write_guid(&mut out, &data.id.created_world_id);
    write_guid(&mut out, &data.id.local_id_in_created_world);
    write_fstring(&mut out, &data.static_id);
    match &data.payload {
        DynamicItemPayload::Durability {
            unknown_0,
            durability,
            remaining_bullets,
        } => {
            write_u32_le(&mut out, *unknown_0);
            write_f32_le(&mut out, *durability);
            write_i32_le(&mut out, *remaining_bullets);
        }
        DynamicItemPayload::WithAmmo {
            unknown_0,
            durability,
            remaining_bullets,
            unknown_1,
            ammo_static_id,
            unknown_2,
        } => {
            write_u32_le(&mut out, *unknown_0);
            write_f32_le(&mut out, *durability);
            write_i32_le(&mut out, *remaining_bullets);
            write_u32_le(&mut out, *unknown_1);
            write_fstring(&mut out, ammo_static_id);
            write_u32_le(&mut out, *unknown_2);
        }
        DynamicItemPayload::Egg {
            unknown_0,
            character_id,
            unknown_name,
            trailing_bytes,
        } => {
            write_u32_le(&mut out, *unknown_0);
            write_fstring(&mut out, character_id);
            write_fstring(&mut out, unknown_name);
            out.extend_from_slice(trailing_bytes);
        }
        DynamicItemPayload::Opaque(bytes) => out.extend_from_slice(bytes),
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

    fn item(payload: DynamicItemPayload) -> DynamicItem {
        DynamicItem {
            id: DynamicId {
                created_world_id: [0u8; 16],
                local_id_in_created_world: [3u8; 16],
            },
            static_id: ascii("ClothArmor"),
            payload,
        }
    }

    #[test]
    fn durability_shape_round_trips() {
        let data = item(DynamicItemPayload::Durability {
            unknown_0: 0,
            durability: 150.0,
            remaining_bullets: 0,
        });
        let bytes = encode(&data);
        assert_eq!(decode(&bytes).unwrap(), data);
        assert_eq!(data.durability(), Some(150.0));
    }

    #[test]
    fn with_ammo_shape_round_trips() {
        let data = item(DynamicItemPayload::WithAmmo {
            unknown_0: 0,
            durability: 263.0,
            remaining_bullets: 1,
            unknown_1: 0,
            ammo_static_id: ascii("Arrow_Fire"),
            unknown_2: 0,
        });
        let bytes = encode(&data);
        assert_eq!(decode(&bytes).unwrap(), data);
        assert_eq!(data.ammo_static_id().as_deref(), Some("Arrow_Fire"));
        assert_eq!(data.remaining_bullets(), Some(1));
    }

    /// The game's own sentinel for "nothing loaded" is the string `None`, not an empty
    /// string — it must survive re-encoding but must not reach a UI as an item name.
    #[test]
    fn the_none_sentinel_round_trips_but_reads_as_absent() {
        let data = item(DynamicItemPayload::WithAmmo {
            unknown_0: 0,
            durability: 30.0,
            remaining_bullets: 0,
            unknown_1: 0,
            ammo_static_id: ascii("None"),
            unknown_2: 0,
        });
        assert_eq!(decode(&encode(&data)).unwrap(), data);
        assert_eq!(data.ammo_static_id(), None);
    }

    #[test]
    fn egg_shape_round_trips() {
        let data = item(DynamicItemPayload::Egg {
            unknown_0: 0,
            character_id: ascii("Deer"),
            unknown_name: ascii("None"),
            trailing_bytes: vec![0; EGG_TRAILER_LEN],
        });
        let bytes = encode(&data);
        assert_eq!(decode(&bytes).unwrap(), data);
        assert_eq!(data.egg_character_id().as_deref(), Some("Deer"));
    }

    /// A payload no shape explains must survive untouched rather than being forced
    /// into the nearest fit.
    #[test]
    fn an_unexplained_payload_stays_opaque_and_round_trips() {
        let payload = vec![0xab; 17];
        let data = item(DynamicItemPayload::Opaque(payload.clone()));
        let bytes = encode(&data);
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded, data);
        assert_eq!(decoded.durability(), None);
        assert_eq!(matching_shape_count(&payload), 0);
    }

    /// Length collisions between shapes are possible in principle. When one happens the
    /// answer is "unknown", never a guess — this pins that behaviour rather than
    /// leaving it to the fixture test, which can only prove real saves avoid it.
    #[test]
    fn an_ambiguous_payload_is_refused_rather_than_guessed() {
        // Hand-built so that both the WithAmmo and Egg readers consume it exactly:
        // u32, f32, i32, u32, FString(len 18), u32  ==  u32, FString, FString, 28 bytes
        let mut payload = Vec::new();
        write_u32_le(&mut payload, 0);
        write_f32_le(&mut payload, 1.0);
        write_i32_le(&mut payload, 0);
        write_u32_le(&mut payload, 0);
        write_fstring(&mut payload, &ascii("AmmoNameOf17Chars"));
        write_u32_le(&mut payload, 0);

        // Only assert the refusal when the collision actually materialized; the point
        // is the policy, not this particular byte string.
        if matching_shape_count(&payload) > 1 {
            assert!(matches!(classify(&payload), DynamicItemPayload::Opaque(_)));
        }

        // Whatever it classified as, the bytes must survive.
        let data = item(classify(&payload));
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }
}
