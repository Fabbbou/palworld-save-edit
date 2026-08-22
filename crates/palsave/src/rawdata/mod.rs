//! Palworld-specific decoders for the opaque `RawData` byte blobs the generic GVAS
//! layer (`crate::gvas`) can't see into. One module per path. Ported from
//! `oMaN-Rod/uesave-rs` (branch `pluggable-game-support`, MIT) — see ADR-002.md for
//! why that's the reference used, rather than the older cheahjs/palworld-save-tools
//! the project plan originally named. Unlisted paths are deliberately left as opaque
//! bytes — see `CLAUDE.md`'s Scope section.

pub mod character;
pub mod error;
pub mod group;
pub mod item_container;
