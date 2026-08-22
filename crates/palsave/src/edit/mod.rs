//! Splice engine: apply edits without re-serializing the save.
//!
//! Every edit is expressed as a set of byte-range replacements against the original
//! decompressed GVAS buffer. `SpliceSet::apply` sorts them, rejects overlaps, and
//! assembles the result as a chunk list plus one final copy — untouched regions are
//! `memcpy`'d straight from the source and never re-encoded. Editing one guild
//! rewrites that guild's `RawData` blob and patches a handful of 4-byte `size`
//! fields; the multi-megabyte `MapObjectSaveData` sibling is copied verbatim.
//!
//! ## Why size fixups are the whole problem
//!
//! A GVAS property tag carries a u32 `size`: the byte length of the value that
//! follows it. Change a value's length and every *enclosing* property's `size` is
//! now wrong, all the way up to the top-level property. Nothing else in the format
//! needs adjusting for an in-place value edit — struct bodies are None-terminated
//! (no length prefix), and map entries have no per-entry length either — so the
//! complete fixup is: patch the `size` field of the edited property and of each of
//! its ancestors by the same delta. `replace_property_value` does exactly that,
//! given the ancestor chain.
//!
//! Adding or removing map entries or array elements additionally changes a count
//! field and is deliberately not supported here yet; `replace_property_value` only
//! swaps one value for another.

pub mod error;

pub use error::EditError;

use crate::gvas::PropertyEntry;
use crate::gvas::property::{size_field_offset, value_offset};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct SpliceSet {
    splices: Vec<Splice>,
}

impl SpliceSet {
    pub fn new() -> Self {
        SpliceSet {
            splices: Vec::new(),
        }
    }

    pub fn replace(&mut self, range: Range<usize>, bytes: Vec<u8>) {
        self.splices.push(Splice { range, bytes });
    }

    pub fn is_empty(&self) -> bool {
        self.splices.is_empty()
    }

    pub fn merge(&mut self, other: SpliceSet) {
        self.splices.extend(other.splices);
    }

    /// Assembles the edited buffer. Splices are applied in source order; overlapping
    /// ranges are rejected rather than silently resolved, since which one "wins"
    /// would be arbitrary and the result would be a corrupt save.
    pub fn apply(mut self, source: &[u8]) -> Result<Vec<u8>, EditError> {
        self.splices.sort_by_key(|s| s.range.start);

        for pair in self.splices.windows(2) {
            if pair[0].range.end > pair[1].range.start {
                return Err(EditError::OverlappingSplices {
                    first: pair[0].range.clone(),
                    second: pair[1].range.clone(),
                });
            }
        }
        if let Some(last) = self.splices.last()
            && last.range.end > source.len()
        {
            return Err(EditError::SpliceOutOfBounds {
                range: last.range.clone(),
                source_len: source.len(),
            });
        }

        let mut out = Vec::with_capacity(source.len());
        let mut cursor = 0usize;
        for splice in &self.splices {
            out.extend_from_slice(&source[cursor..splice.range.start]);
            out.extend_from_slice(&splice.bytes);
            cursor = splice.range.end;
        }
        out.extend_from_slice(&source[cursor..]);
        Ok(out)
    }
}

/// Encodes a `TArray<uint8>` property value: a u32 element count followed by the
/// bytes themselves. This is the wire shape of every Palworld `RawData` property, so
/// it's what a re-encoded `rawdata` blob has to be wrapped in before splicing.
pub fn byte_array_value(blob: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + blob.len());
    out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
    out.extend_from_slice(blob);
    out
}

/// Replaces one property's entire value region and patches the `size` field of that
/// property's own tag plus every ancestor's.
///
/// `chain` runs outermost-first and must end with the property being edited — e.g.
/// `[worldSaveData, GroupSaveDataMap, RawData]`. Each entry must physically contain
/// the next; that's checked, not assumed, because a caller assembling the chain from
/// separate `materialize` calls could easily pass siblings by mistake and the
/// resulting save would be quietly corrupt.
pub fn replace_property_value(
    source: &[u8],
    chain: &[&PropertyEntry],
    new_value: Vec<u8>,
    has_property_guid: bool,
) -> Result<SpliceSet, EditError> {
    let Some(leaf) = chain.last() else {
        return Err(EditError::EmptyChain);
    };

    for pair in chain.windows(2) {
        let (outer, inner) = (&pair[0].span, &pair[1].span);
        if !(outer.start <= inner.start && inner.end <= outer.end) {
            return Err(EditError::NotNested {
                outer: outer.clone(),
                inner: inner.clone(),
            });
        }
    }

    let value_start = value_offset(source, leaf.span.start, has_property_guid)?;
    let old_value_len = leaf.span.end - value_start;
    let delta = new_value.len() as i64 - old_value_len as i64;

    let mut set = SpliceSet::new();
    set.replace(value_start..leaf.span.end, new_value);

    if delta != 0 {
        for entry in chain {
            let offset = size_field_offset(source, entry.span.start)?;
            let old_size = u32::from_le_bytes(
                source
                    .get(offset..offset + 4)
                    .ok_or(EditError::SpliceOutOfBounds {
                        range: offset..offset + 4,
                        source_len: source.len(),
                    })?
                    .try_into()
                    .unwrap(),
            );
            let new_size = i64::from(old_size) + delta;
            let new_size = u32::try_from(new_size).map_err(|_| EditError::SizeOutOfRange {
                offset,
                old_size,
                delta,
            })?;
            set.replace(offset..offset + 4, new_size.to_le_bytes().to_vec());
        }
    }

    Ok(set)
}

/// Structural verification for an edited buffer, to be run before handing bytes back
/// to a caller. Re-parses the GVAS and asserts the parse partitions the buffer
/// exactly — which is only true if every `size` field agrees with the actual byte
/// layout, at every level. A missed or miscomputed fixup fails here rather than
/// silently producing a save the game will reject or misread.
///
/// `CLAUDE.md`: "Losing a world to a silent encoder bug is the failure mode that
/// actually matters." This is the check that makes that failure loud.
pub fn verify_reparses(bytes: &[u8]) -> Result<(), EditError> {
    let file = crate::gvas::GvasFile::parse(bytes)?;
    if file.write() != bytes {
        return Err(EditError::VerificationFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_splices_in_source_order() {
        let source = b"hello world".to_vec();
        let mut set = SpliceSet::new();
        // Deliberately out of order — apply() sorts.
        set.replace(6..11, b"there".to_vec());
        set.replace(0..5, b"HI".to_vec());
        assert_eq!(set.apply(&source).unwrap(), b"HI there".to_vec());
    }

    #[test]
    fn apply_with_no_splices_is_the_identity() {
        let source = b"unchanged".to_vec();
        assert_eq!(SpliceSet::new().apply(&source).unwrap(), source);
    }

    #[test]
    fn overlapping_splices_are_rejected() {
        let source = b"hello world".to_vec();
        let mut set = SpliceSet::new();
        set.replace(0..6, b"a".to_vec());
        set.replace(3..8, b"b".to_vec());
        assert!(matches!(
            set.apply(&source),
            Err(EditError::OverlappingSplices { .. })
        ));
    }

    #[test]
    fn out_of_bounds_splice_is_rejected() {
        let source = b"short".to_vec();
        let mut set = SpliceSet::new();
        set.replace(0..99, b"x".to_vec());
        assert!(matches!(
            set.apply(&source),
            Err(EditError::SpliceOutOfBounds { .. })
        ));
    }

    #[test]
    fn byte_array_value_prefixes_the_count() {
        assert_eq!(byte_array_value(&[7, 8, 9]), vec![3, 0, 0, 0, 7, 8, 9]);
        assert_eq!(byte_array_value(&[]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn empty_chain_is_rejected() {
        assert!(matches!(
            replace_property_value(b"", &[], vec![], true),
            Err(EditError::EmptyChain)
        ));
    }

    #[test]
    fn non_nested_chain_is_rejected() {
        let outer = PropertyEntry {
            name: "a".into(),
            type_name: "IntProperty".into(),
            span: 0..10,
        };
        let sibling = PropertyEntry {
            name: "b".into(),
            type_name: "IntProperty".into(),
            span: 20..30,
        };
        let err =
            replace_property_value(&[0u8; 64], &[&outer, &sibling], vec![], true).unwrap_err();
        assert!(matches!(err, EditError::NotNested { .. }));
    }
}
