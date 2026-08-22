//! `.worldSaveData.ItemContainerSaveData` RawData blobs: inventories. Two distinct
//! shapes at two different paths, both ported from `oMaN-Rod/uesave-rs` (branch
//! `pluggable-game-support`, MIT), `uesave/src/games/palworld/items.rs` — the same
//! actively-maintained fork ADR-002.md and ADR-003.md credit for `GroupSaveDataMap`
//! and `CharacterSaveParameterMap`:
//!
//! - `.Value.RawData` (`PalItemContainer`): the container's own permissions — which
//!   item types it accepts. `decode_container`/`encode_container`.
//! - `.Value.Slots[].RawData` (`PalItemContainerSlot`): one inventory slot's item and
//!   count. `decode_slot`/`encode_slot`.
//!
//! Both keep their tail as an unparsed byte blob rather than a fixed-size field —
//! `ar.read_to_end(..)` upstream, `bytes[pos..].to_vec()` here — so unlike
//! `GroupSaveDataMap`'s fixed-width trailers, neither of these has a hardcoded size
//! that could silently mismatch after the next format drift. There's nothing to
//! verify-by-EOF here the way `group`/`character` do: reading to end always
//! "succeeds" by construction, so there's no failure mode to guard against beyond the
//! ordinary bounds checks on the fixed-size fields before it.

use super::error::RawDataError;
use crate::gvas::primitives::{
    FString, Guid, read_fstring, read_guid, read_i32_le, read_u8, read_u32_le, write_fstring,
    write_guid, write_i32_le, write_u32_le,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ItemContainer {
    pub type_a: Vec<u8>,
    pub type_b: Vec<u8>,
    pub item_static_ids: Vec<FString>,
    pub trailing_unparsed_data: Vec<u8>,
}

pub fn decode_container(bytes: &[u8]) -> Result<ItemContainer, RawDataError> {
    let mut pos = 0usize;

    let type_a_count = read_u32_le(bytes, &mut pos)?;
    let mut type_a = Vec::with_capacity(type_a_count as usize);
    for _ in 0..type_a_count {
        type_a.push(read_u8(bytes, &mut pos)?);
    }

    let type_b_count = read_u32_le(bytes, &mut pos)?;
    let mut type_b = Vec::with_capacity(type_b_count as usize);
    for _ in 0..type_b_count {
        type_b.push(read_u8(bytes, &mut pos)?);
    }

    let item_static_ids_count = read_u32_le(bytes, &mut pos)?;
    let mut item_static_ids = Vec::with_capacity(item_static_ids_count as usize);
    for _ in 0..item_static_ids_count {
        item_static_ids.push(read_fstring(bytes, &mut pos)?);
    }

    let trailing_unparsed_data = bytes[pos..].to_vec();

    Ok(ItemContainer {
        type_a,
        type_b,
        item_static_ids,
        trailing_unparsed_data,
    })
}

pub fn encode_container(data: &ItemContainer) -> Vec<u8> {
    let mut out = Vec::new();
    write_u32_le(&mut out, data.type_a.len() as u32);
    out.extend_from_slice(&data.type_a);
    write_u32_le(&mut out, data.type_b.len() as u32);
    out.extend_from_slice(&data.type_b);
    write_u32_le(&mut out, data.item_static_ids.len() as u32);
    for id in &data.item_static_ids {
        write_fstring(&mut out, id);
    }
    out.extend_from_slice(&data.trailing_unparsed_data);
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicId {
    pub created_world_id: Guid,
    pub local_id_in_created_world: Guid,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemId {
    pub static_id: FString,
    pub dynamic_id: DynamicId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemContainerSlot {
    pub slot_index: i32,
    pub count: i32,
    pub item: ItemId,
    pub trailing_bytes: Vec<u8>,
}

pub fn decode_slot(bytes: &[u8]) -> Result<ItemContainerSlot, RawDataError> {
    let mut pos = 0usize;
    let slot_index = read_i32_le(bytes, &mut pos)?;
    let count = read_i32_le(bytes, &mut pos)?;
    let static_id = read_fstring(bytes, &mut pos)?;
    let created_world_id = read_guid(bytes, &mut pos)?;
    let local_id_in_created_world = read_guid(bytes, &mut pos)?;
    let trailing_bytes = bytes[pos..].to_vec();

    Ok(ItemContainerSlot {
        slot_index,
        count,
        item: ItemId {
            static_id,
            dynamic_id: DynamicId {
                created_world_id,
                local_id_in_created_world,
            },
        },
        trailing_bytes,
    })
}

pub fn encode_slot(data: &ItemContainerSlot) -> Vec<u8> {
    let mut out = Vec::new();
    write_i32_le(&mut out, data.slot_index);
    write_i32_le(&mut out, data.count);
    write_fstring(&mut out, &data.item.static_id);
    write_guid(&mut out, &data.item.dynamic_id.created_world_id);
    write_guid(&mut out, &data.item.dynamic_id.local_id_in_created_world);
    out.extend_from_slice(&data.trailing_bytes);
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

    #[test]
    fn container_round_trips() {
        let data = ItemContainer {
            type_a: vec![1, 2, 3],
            type_b: vec![],
            item_static_ids: vec![ascii("Wood"), ascii("Stone")],
            trailing_unparsed_data: vec![9, 9],
        };
        let bytes = encode_container(&data);
        assert_eq!(decode_container(&bytes).unwrap(), data);
    }

    #[test]
    fn container_with_no_trailing_data_round_trips() {
        let data = ItemContainer {
            type_a: vec![],
            type_b: vec![],
            item_static_ids: vec![],
            trailing_unparsed_data: vec![],
        };
        let bytes = encode_container(&data);
        assert_eq!(decode_container(&bytes).unwrap(), data);
    }

    #[test]
    fn slot_round_trips() {
        let data = ItemContainerSlot {
            slot_index: 3,
            count: 12,
            item: ItemId {
                static_id: ascii("Wood"),
                dynamic_id: DynamicId {
                    created_world_id: [1u8; 16],
                    local_id_in_created_world: [0u8; 16],
                },
            },
            trailing_bytes: vec![7, 7, 7],
        };
        let bytes = encode_slot(&data);
        assert_eq!(decode_slot(&bytes).unwrap(), data);
    }
}
