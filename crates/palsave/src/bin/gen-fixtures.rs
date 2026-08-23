//! Writes synthetic `.sav` files for the browser end-to-end suite.
//!
//! Real saves are gitignored (SteamIDs, player names), so CI has nothing to drive the
//! UI with. This emits structurally real stand-ins built by `palsave::synthetic` —
//! the same builders the wasm boundary tests use, so there is one definition of what
//! a synthetic save is rather than two that can drift.
//!
//! Usage: `cargo run --bin gen-fixtures --features synthetic -- <out-dir>`

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(out_dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: gen-fixtures <out-dir>");
        return ExitCode::FAILURE;
    };

    // Two worlds, because migration is a two-world question and the preview screen
    // cannot be exercised with one. `other/` is WORLD_B: different player, different
    // guild, different containers — but the same Pal instance id, reproducing the real
    // collision found in the fixture corpus.
    let players_dir = out_dir.join("Players");
    let other_dir = out_dir.join("other");
    let other_players_dir = other_dir.join("Players");
    for dir in [&players_dir, &other_players_dir] {
        if let Err(e) = fs::create_dir_all(dir) {
            eprintln!("failed to create {}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    use palsave::synthetic::{WORLD_A, WORLD_B};
    let uid = palsave::gvas::nav::guid_to_hex(&WORLD_A.player_uid);
    let other_uid = palsave::gvas::nav::guid_to_hex(&WORLD_B.player_uid);
    let files = [
        (
            out_dir.join("Level.sav"),
            palsave::synthetic::synthetic_sav(),
        ),
        (
            players_dir.join(format!("{uid}.sav")),
            palsave::synthetic::synthetic_player_sav(),
        ),
        (
            other_dir.join("Level.sav"),
            palsave::synthetic::synthetic_sav_for(&WORLD_B),
        ),
        (
            other_players_dir.join(format!("{other_uid}.sav")),
            palsave::synthetic::synthetic_player_sav_for(&WORLD_B),
        ),
    ];

    for (path, bytes) in files {
        if let Err(e) = fs::write(&path, &bytes) {
            eprintln!("failed to write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
        println!("{} ({} bytes)", path.display(), bytes.len());
    }

    ExitCode::SUCCESS
}
