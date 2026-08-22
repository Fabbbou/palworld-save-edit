# Project invariants

## The boundary rule — most important thing in this repo
The save data NEVER crosses into JavaScript. Rust owns the decompressed buffer in
linear memory. JS holds an opaque handle. All reads return small view models
(a list of guilds; one Pal's stats). All writes are commands sent in.
Serializing the tree across wasm-bindgen as JSON rebuilds the exact memory disaster
that broke palworld.tf, with extra steps. If a function signature returns anything
resembling the whole save, it's wrong.

## Correctness
- The round-trip test defines correct: parse(bytes) -> write() must be byte-identical
  GVAS for every fixture. Nothing else matters until this is green.
- Compare DECOMPRESSED GVAS bytes, never the .sav container. Recompression is not
  byte-stable and never will be (Oodle is decompress-only; we downgrade to zlib).
- Never guess a format detail. Add a fixture and verify. A plausible-looking wrong
  answer here destroys 200-hour worlds.

## Memory
- wasm32 has a 4GB address space ceiling. Hold exactly ONE copy of the decompressed
  buffer. Parse lazily into (offset, len) spans; borrow, don't clone.
- Decode a subtree only when the user edits it. Re-encode only what changed.

## Architecture
- crates/palsave is pure Rust with no wasm-bindgen. It must be testable with
  `cargo test` natively against fixtures. Fast loop, no browser.
- crates/palsave-wasm is a thin binding layer only. No logic.

## Scope
- Unknown regions pass through as opaque bytes. That is a feature.
- No UI work while a round-trip test is red.

## Privacy
- No network at runtime. No analytics. No telemetry.
- Fixtures contain SteamIDs and player names. fixtures/ is gitignored, always.

## Anti-goals
- Never serialize the save tree across the wasm boundary. Handles and view models only.
- Do not attempt Oodle compression. No open-source implementation exists. Loading a
  system `oo2core_*.dll` works on desktop and is impossible in a browser.
- Do not enable wasm threads / rayon. COOP/COEP can't be set on static hosting.
- Do not depend on uesave-rs at runtime. It parses eagerly; that defeats the point.
  Dev-dependency only, as a differential-test oracle.
- Do not decode MapObjectSaveData or the foliage maps.
- Do not put logic in crates/palsave-wasm.
- Do not commit fixtures.
- Do not build UI while a round-trip test is red.
