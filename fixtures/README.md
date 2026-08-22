# fixtures

Gitignored — real save files, always contain SteamIDs and player names.

Collect for local dev/testing, one save dir per source:

```
fixtures/<label>/Players/<uid>.sav
fixtures/<label>/Level.sav
fixtures/<label>/LevelMeta.sav
fixtures/<label>/WorldOption.sav
```

Want at minimum:

- one current-format save (PlM / Oodle) — the format every existing tool is broken on
- one older PlZ-era save, if you still have one
- one Game Pass / WGS save (CNK wrapper) if you can get one

Run `cargo run --bin sniff -- fixtures/<label>/Level.sav` against each to confirm the
container before anything downstream depends on it.
