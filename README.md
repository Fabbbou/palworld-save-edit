# palworld-save-edit

A Palworld save editor that runs entirely in your browser. Rust core compiled to
WebAssembly, Svelte 5 UI, fully static — no server, no uploads, no telemetry.

The privacy claim is structural rather than a promise: there are no network calls at
runtime. Your save is read into wasm memory in your own tab and never leaves it.

## Status

Opens `.sav` files — including the current **PlM / Oodle** format that most existing
open-source tools can't read — and exports edits as a new file.

**Reading:** the world's guilds and members; every player and Pal with stats, IVs and
passive skills; each player's six inventories; their party and Pal box, showing which
Pal is in which slot; per-item durability, loaded ammunition, and what's inside an egg.

**Editing:** guild names, Pal stats (level, exp, the three IVs), Pal nicknames, and
player level and exp.

**Not yet:** editing inventory or Pal-box contents, migrating a player between saves,
the raw property tree browser, and in-place write-back with automatic backup. Anything
stored in `Players/<uid>.sav` is read-only for now — the export path only rewrites
`Level.sav`.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for layout, commands, and how to add a fixture
when the game format changes. Design decisions are recorded in `ADR-00*.md`;
`CLAUDE.md` holds the invariants.

```bash
npm install
npm run dev
```

Requires a Rust toolchain with the `wasm32-unknown-unknown` target.

## How it works

| Concern | Choice | Why |
|---|---|---|
| Core logic | Rust → `wasm32-unknown-unknown` | `u64` is `u64`. No BigInt discipline, no 2^53 corruption class. |
| Oodle | [`oozextract`](https://crates.io/crates/oozextract) | Pure Rust, compiles to wasm with no C++ toolchain (see ADR-001.md). |
| GVAS parsing | hand-written, lazy, span-based | Parse the top-level index; decode a subtree only when it's needed. |
| Editing | byte-range splice engine | An edit rewrites one subtree and patches enclosing size fields; everything else is `memcpy` (see ADR-004.md). |
| UI | Vite + Svelte 5 runes | No SSR layer; `vite build` produces a static `dist/`. |
| Threading | single-threaded | wasm threads need COOP/COEP headers that static hosts can't set. |

The save never crosses into JavaScript. Rust owns the buffer; JS holds an opaque
handle and receives small view models. See ADR-005.md.

## Format compatibility

Palworld's save format is undocumented and changes with the game. This project's
defence is that every decoder either consumes its input *exactly* or refuses — a
region we don't fully understand is passed through as opaque bytes rather than
half-parsed. ADR-002.md is a worked example of catching a real format drift that way.

Verified against engine 5.1.1 saves in the PlM container.

## Credits

Format knowledge is derived from two MIT-licensed projects, both credited in detail in
the ADRs:

- [`oMaN-Rod/uesave-rs`](https://github.com/oMaN-Rod/uesave-rs) (branch
  `pluggable-game-support`) — the current, actively maintained Palworld `RawData`
  layouts and type-hint table.
- [`trumank/uesave-rs`](https://github.com/trumank/uesave-rs) — generic Unreal GVAS
  semantics, and this project's differential-test oracle.
- [`cheahjs/palworld-save-tools`](https://github.com/cheahjs/palworld-save-tools) —
  the original format documentation, still the best reference for the property layer.

## Licence

Not yet chosen. The format work above is MIT-licensed and credited accordingly.
