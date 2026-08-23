//! Palworld-specific decoders for the opaque `RawData` byte blobs the generic GVAS
//! layer (`crate::gvas`) can't see into. One module per path. Ported from
//! `oMaN-Rod/uesave-rs` (branch `pluggable-game-support`, MIT) — see ADR-002.md for
//! why that's the reference used, rather than the older cheahjs/palworld-save-tools
//! the project plan originally named. Unlisted paths are deliberately left as opaque
//! bytes — see `CLAUDE.md`'s Scope section.
//!
//! `character_container` is the exception to the porting note: it was measured
//! directly from two real worlds with `examples/dump_blobs`, because the shape older
//! references describe is contradicted by the current save format. Its module docs
//! record what was measured and what stays unnamed.

pub mod character;
pub mod character_container;
pub mod error;
pub mod group;
pub mod item_container;
