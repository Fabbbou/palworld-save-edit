//! Dev tool: list top-level GVAS properties (name, type, byte span) for one or more
//! .sav files. Not part of the public API — just a quick way to eyeball what a
//! fixture's index looks like without writing a throwaway test.
//!
//! Usage: cargo run --example inspect -p palsave -- <path> [path...]

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: inspect <path-to-.sav> [path...]");
        std::process::exit(1);
    }

    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{path}: read failed: {e}");
                continue;
            }
        };
        let container = match palsave::container::decode(&bytes) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{path}: container decode failed: {e}");
                continue;
            }
        };
        let file = match palsave::gvas::GvasFile::parse(&container.gvas) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("{path}: GVAS parse failed: {e}");
                continue;
            }
        };

        println!(
            "{path}: engine {}.{}.{}, {}, {} top-level properties",
            file.header.engine_version_major,
            file.header.engine_version_minor,
            file.header.engine_version_patch,
            file.save_game_type,
            file.properties.len(),
        );
        for p in &file.properties {
            println!("  {} : {} ({} bytes)", p.name, p.type_name, p.span.len());
        }
    }
}
