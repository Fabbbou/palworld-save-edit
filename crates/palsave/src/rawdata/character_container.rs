//! `.worldSaveData.CharacterContainerSaveData.Value.Slots[].RawData`: which Pal sits
//! in which Pal-box or party slot.
//!
//! The map's outer shape is the same family as `ItemContainerSaveData` —
//! `bReferenceSlot` / `Slots` / `SlotNum` / `RawData` / `CustomVersionData`, keyed by
//! a `{ID: Guid}` struct — so `gvas::hints` already routes it correctly and
//! `world::open_map` materializes it without help. Only the per-slot blob needed
//! decoding.
//!
//! ## What was measured, and what is still unnamed
//!
//! Unlike the rest of this module's decoders, this one was not ported from
//! `oMaN-Rod/uesave-rs`; it was measured with `examples/dump_blobs` against two
//! unrelated worlds. Every one of the 437 slots across both is **exactly 38 bytes**,
//! with only bytes 16..32 differing between slots:
//!
//! ```text
//!   0000  00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00   <- identical everywhere
//!   0010  90 fb 75 8f b3 25 2a 49 ab d6 d6 b5 d6 54 4d 1c   <- the Pal's instance id
//!   0020  00 00 00 00 00 00                                 <- zero everywhere
//! ```
//!
//! The leading 16 bytes are **not** the owning player's uid, which is the obvious
//! guess and the shape older reference implementations describe. In the two-player
//! world all 301 slots carry the identical value, including the 61 slots of the second
//! player's box — so it cannot identify an owner. Rather than name it on a hypothesis
//! that the data contradicts, it passes through opaquely; ownership already has a
//! verified source in `CharacterSaveParameterMap`'s `OwnerPlayerUId`, which
//! `characters::pals_of` reads.
//!
//! ## No slot index
//!
//! An `ItemContainerSlot` stores its own `slot_index`; this blob has no room for one
//! (16 + 16 + 6 = 38, all accounted for). A container's `Slots` array is shorter than
//! its declared `SlotNum` — 108 of 960 in the reference world — so the array holds
//! occupied slots only and position is the index. That is an observation about the
//! data, not a documented invariant, which is why `PalContainerSlot` carries no index
//! field and callers use the array position.
//!
//! The tail is read to end rather than into a fixed `[u8; 6]`, following
//! `item_container` rather than `character`: a future format that appends a field
//! still yields a correct `instance_id` (it sits at a fixed offset *before* the tail)
//! instead of failing the whole container. The fixture tests pin the exact 38 bytes,
//! so drift is still caught — by a test, where it belongs, rather than by blanking a
//! user's Pal box.

use super::error::RawDataError;
use crate::gvas::primitives::{Guid, read_guid, write_guid};

/// One occupied slot of a Pal box, party, or base-camp Pal container.
#[derive(Debug, Clone, PartialEq)]
pub struct PalContainerSlot {
    /// Constant `00×12 01 00 00 00` in every slot of every world seen so far. See the
    /// module docs for why this is not called `player_uid`.
    pub leading_bytes: [u8; 16],
    /// Joins to a `CharacterSaveParameterMap` entry — the Pal itself.
    pub instance_id: Guid,
    pub trailing_bytes: Vec<u8>,
}

pub fn decode_slot(bytes: &[u8]) -> Result<PalContainerSlot, RawDataError> {
    let mut pos = 0usize;
    let leading_bytes = read_guid(bytes, &mut pos)?;
    let instance_id = read_guid(bytes, &mut pos)?;
    let trailing_bytes = bytes[pos..].to_vec();

    Ok(PalContainerSlot {
        leading_bytes,
        instance_id,
        trailing_bytes,
    })
}

pub fn encode_slot(data: &PalContainerSlot) -> Vec<u8> {
    let mut out = Vec::with_capacity(38);
    write_guid(&mut out, &data.leading_bytes);
    write_guid(&mut out, &data.instance_id);
    out.extend_from_slice(&data.trailing_bytes);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_round_trips() {
        let data = PalContainerSlot {
            leading_bytes: [1u8; 16],
            instance_id: [2u8; 16],
            trailing_bytes: vec![0; 6],
        };
        let bytes = encode_slot(&data);
        assert_eq!(bytes.len(), 38);
        assert_eq!(decode_slot(&bytes).unwrap(), data);
    }

    /// The tail is deliberately variable-length, so a longer blob must round-trip
    /// rather than error — that is what keeps an appended field from blanking a
    /// container. See the module docs.
    #[test]
    fn a_longer_tail_round_trips() {
        let data = PalContainerSlot {
            leading_bytes: [0u8; 16],
            instance_id: [7u8; 16],
            trailing_bytes: vec![9; 11],
        };
        let bytes = encode_slot(&data);
        assert_eq!(decode_slot(&bytes).unwrap(), data);
    }

    #[test]
    fn a_truncated_slot_is_rejected() {
        // 31 bytes: not even the two guids fit.
        assert!(decode_slot(&[0u8; 31]).is_err());
    }
}
