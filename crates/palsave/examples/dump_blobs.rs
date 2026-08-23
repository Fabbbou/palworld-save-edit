//! Dev tool: hex-dumps RawData blobs from any `worldSaveData` map or array, so a
//! layout can be *measured* before a decoder is written for it.
//!
//! `examples/dump_group.rs` did this for `GroupSaveDataMap` alone, hardcoded. Every
//! new decoder in this crate started with someone staring at those bytes, so the
//! staring tool is worth generalizing rather than copying a fifth time.
//!
//! Usage:
//!   dump_blobs <path> map   <MapName>   [count]   # .Value.RawData per map entry
//!   dump_blobs <path> slots <MapName>   [count]   # .Value.Slots[].RawData per slot
//!   dump_blobs <path> array <ArrayName> [count]   # .RawData per array element
//!   dump_blobs <path> tails <ArrayName> [count]   # array, but histogram the payload
//!                                                 # left after Guid,Guid,FString
//!
//! `count` caps how many blobs are printed (default 4). Sizes are always summarized
//! for *every* blob, because "is this field fixed-width?" is answered by the spread of
//! lengths across the whole map, not by the first four.
//!
//! `tails` exists for the same reason: `DynamicItemSaveData` blobs vary in total
//! length purely because they embed an item id, so raw lengths say nothing. Stripping
//! the known prefix first is what reveals whether the remaining payload has one shape
//! or several.

use palsave::gvas::primitives::{read_fstring, read_guid};
use palsave::gvas::value::{Value, materialize_property};
use palsave::gvas::{GvasFile, PropertyEntry};
use palsave::world;
use std::collections::BTreeMap;

/// 16 bytes per line, offset + hex + ASCII gutter. The ASCII column is what makes an
/// embedded FString jump out of an otherwise anonymous run of bytes.
fn hex_dump(bytes: &[u8]) {
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let offset = row * 16;
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        println!("  {offset:04x}  {:<47}  |{ascii}|", hex.join(" "));
    }
}

/// Min/max/distinct-count over blob lengths. A single distinct length across hundreds
/// of entries is strong evidence of a fixed-width record; a spread means something
/// variable-length (an FString, a list) is in there.
fn summarize(label: &str, lengths: &[usize]) {
    if lengths.is_empty() {
        println!("{label}: no blobs found");
        return;
    }
    let mut distinct: Vec<usize> = lengths.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    let shown: Vec<String> = distinct.iter().take(8).map(|l| l.to_string()).collect();
    let more = if distinct.len() > 8 {
        format!(" ... and {} more", distinct.len() - 8)
    } else {
        String::new()
    };
    println!(
        "{label}: {} blobs, {} distinct length(s): [{}]{more}",
        lengths.len(),
        distinct.len(),
        shown.join(", ")
    );
}

fn raw_data_of(
    gvas: &[u8],
    fields: &[PropertyEntry],
    engine_major: u16,
    has_property_guid: bool,
    path: &str,
) -> Option<Vec<u8>> {
    let entry = fields.iter().find(|f| f.name == "RawData")?;
    let value = materialize_property(gvas, entry, engine_major, has_property_guid, path).ok()?;
    value.as_bytes().map(|b| b.to_vec())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: dump_blobs <path> <map|slots|array> <Name> [count]");
        std::process::exit(1);
    }
    let (path, mode, name) = (&args[0], args[1].as_str(), &args[2]);
    let limit: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);

    let bytes = std::fs::read(path).expect("read");
    let container = palsave::container::decode(&bytes).expect("container decode");
    let gvas = &container.gvas;
    let file = GvasFile::parse(gvas).expect("gvas parse");
    let engine_major = file.header.engine_version_major;
    let has_property_guid = file.header.has_property_guid();

    let mut lengths = Vec::new();
    let mut tail_lengths: Vec<usize> = Vec::new();
    // One representative payload per distinct tail length, so every shape gets looked
    // at rather than just whichever happened to come first in the array.
    let mut tails_by_len: BTreeMap<usize, (String, Vec<u8>)> = BTreeMap::new();
    let mut printed = 0usize;

    match mode {
        "map" | "slots" => {
            let map = world::open_map(gvas, name).expect("open_map");
            println!("worldSaveData.{name}: {} entries\n", map.entries.len());
            let value_path = format!("worldSaveData.{name}.Value");

            for (i, entry) in map.entries.iter().enumerate() {
                if mode == "map" {
                    let Some(raw) = raw_data_of(
                        gvas,
                        &entry.fields,
                        engine_major,
                        has_property_guid,
                        &format!("{value_path}.RawData"),
                    ) else {
                        continue;
                    };
                    lengths.push(raw.len());
                    if printed < limit {
                        println!("[{i}] .Value.RawData  {} bytes", raw.len());
                        hex_dump(&raw);
                        println!();
                        printed += 1;
                    }
                } else {
                    // A container's declared capacity, alongside how many slots are
                    // actually stored — the two differ, and knowing which one the
                    // Slots array tracks is half of understanding the layout.
                    let slot_num = map
                        .cursor
                        .get_opt(&entry.fields, "SlotNum")
                        .and_then(|v| v.as_integer());
                    let Some(slots) = map
                        .cursor
                        .get_opt(&entry.fields, "Slots")
                        .and_then(|v| v.as_array().map(|a| a.to_vec()))
                    else {
                        continue;
                    };
                    println!("[{i}] SlotNum={slot_num:?} Slots.len()={} ", slots.len());
                    for (j, slot) in slots.iter().enumerate() {
                        let Some(slot_fields) = slot.as_properties() else {
                            continue;
                        };
                        let Some(raw) = raw_data_of(
                            gvas,
                            slot_fields,
                            engine_major,
                            has_property_guid,
                            &format!("{value_path}.Slots.RawData"),
                        ) else {
                            continue;
                        };
                        lengths.push(raw.len());
                        if printed < limit {
                            println!("  [{i}][{j}] .Slots[].RawData  {} bytes", raw.len());
                            hex_dump(&raw);
                            println!();
                            printed += 1;
                        }
                    }
                }
            }
        }
        "array" | "tails" => {
            // No `world::open_array` yet, so walk it here: worldSaveData -> <Name>.
            let world_idx = file
                .properties
                .iter()
                .position(|p| p.name == "worldSaveData")
                .expect("not a Level.sav");
            let world_value = file.materialize(world_idx).expect("materialize");
            let world_children = world_value.as_properties().expect("worldSaveData struct");
            let array_entry = world_children
                .iter()
                .find(|p| p.name == name.as_str())
                .unwrap_or_else(|| panic!("no worldSaveData.{name}"));
            let array_value = materialize_property(
                gvas,
                array_entry,
                engine_major,
                has_property_guid,
                &format!("worldSaveData.{name}"),
            )
            .expect("materialize array");
            let Value::Array(items) = array_value else {
                panic!("worldSaveData.{name} is not an array");
            };
            println!("worldSaveData.{name}: {} elements\n", items.len());

            for (i, item) in items.iter().enumerate() {
                let Some(fields) = item.as_properties() else {
                    continue;
                };
                let Some(raw) = raw_data_of(
                    gvas,
                    fields,
                    engine_major,
                    has_property_guid,
                    &format!("worldSaveData.{name}.RawData"),
                ) else {
                    continue;
                };
                lengths.push(raw.len());

                if mode == "tails" {
                    // Strip the prefix that is already legible — two guids and an
                    // FString — and report only what is left to be explained.
                    let mut pos = 0usize;
                    let id_a = read_guid(&raw, &mut pos);
                    let id_b = read_guid(&raw, &mut pos);
                    let static_id = read_fstring(&raw, &mut pos);
                    let (Ok(id_a), Ok(id_b), Ok(static_id)) = (id_a, id_b, static_id) else {
                        println!("[{i}] prefix did not parse; {} bytes", raw.len());
                        continue;
                    };
                    let tail = &raw[pos..];
                    tail_lengths.push(tail.len());
                    let name = static_id.display_lossy();
                    tails_by_len
                        .entry(tail.len())
                        .or_insert_with(|| (name.clone(), tail.to_vec()));
                    if printed < limit {
                        let zero_a = id_a.iter().all(|&b| b == 0);
                        println!(
                            "[{i}] {name:<24} created_world_id={} tail={} bytes",
                            if zero_a { "zero" } else { "set " },
                            tail.len()
                        );
                        let _ = id_b;
                        hex_dump(tail);
                        printed += 1;
                    }
                    continue;
                }

                if printed < limit {
                    println!("[{i}] .RawData  {} bytes", raw.len());
                    hex_dump(&raw);
                    println!();
                    printed += 1;
                }
            }
        }
        other => {
            eprintln!("unknown mode {other:?}; expected map, slots or array");
            std::process::exit(1);
        }
    }

    summarize(&format!("worldSaveData.{name} ({mode})"), &lengths);

    if mode == "tails" {
        println!();
        summarize(
            &format!("worldSaveData.{name} payload tails"),
            &tail_lengths,
        );
        let mut histogram: BTreeMap<usize, usize> = BTreeMap::new();
        for len in &tail_lengths {
            *histogram.entry(*len).or_insert(0) += 1;
        }
        println!();
        for (len, count) in &histogram {
            let (example, bytes) = &tails_by_len[len];
            println!("tail {len} bytes x{count}  e.g. {example}");
            hex_dump(bytes);
        }
    }
}
