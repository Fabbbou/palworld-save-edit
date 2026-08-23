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

    let players_dir = out_dir.join("Players");
    if let Err(e) = fs::create_dir_all(&players_dir) {
        eprintln!("failed to create {}: {e}", players_dir.display());
        return ExitCode::FAILURE;
    }

    let uid = palsave::gvas::nav::guid_to_hex(&palsave::synthetic::PLAYER_UID);
    let files = [
        (
            out_dir.join("Level.sav"),
            palsave::synthetic::synthetic_sav(),
        ),
        (
            players_dir.join(format!("{uid}.sav")),
            palsave::synthetic::synthetic_player_sav(),
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
