//! Dev tool: materializes one top-level property, then drills into a dotted path of
//! nested struct property names, printing the final value. E.g.:
//!   drill fixtures/Level.sav worldSaveData.GroupSaveDataMap
//!
//! Not part of the public API. Exists to sanity-check `materialize()` on deeply
//! nested real content (Maps, RawData blobs) without loading the whole tree.

use palsave::gvas::value::{StructValue, Value, materialize_property};
use palsave::gvas::{GvasFile, PropertyEntry};

fn find<'a>(props: &'a [PropertyEntry], name: &str) -> Option<&'a PropertyEntry> {
    props.iter().find(|p| p.name == name)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: drill <path> <dotted.property.path>");
    let dotted = args
        .next()
        .expect("usage: drill <path> <dotted.property.path>");

    let bytes = std::fs::read(&path).expect("read");
    let container = palsave::container::decode(&bytes).expect("container decode");
    let file = GvasFile::parse(&container.gvas).expect("gvas parse");

    let mut segments = dotted.split('.');
    let first = segments.next().unwrap();
    let idx = file
        .properties
        .iter()
        .position(|p| p.name == first)
        .unwrap_or_else(|| {
            panic!(
                "no top-level property named {first:?}; have: {:?}",
                file.properties.iter().map(|p| &p.name).collect::<Vec<_>>()
            )
        });

    let mut value = file.materialize(idx).expect("materialize");
    let mut current_path = first.to_string();
    for seg in segments {
        let Value::Struct(StructValue::Properties(props)) = &value else {
            panic!("{seg:?}: parent isn't a generic struct property list, it's {value:?}");
        };
        let entry = find(props, seg).unwrap_or_else(|| {
            panic!(
                "no property named {seg:?}; have: {:?}",
                props.iter().map(|p| &p.name).collect::<Vec<_>>()
            )
        });
        current_path = format!("{current_path}.{seg}");
        value = materialize_property(
            &container.gvas,
            entry,
            file.header.engine_version_major,
            file.header.has_property_guid(),
            &current_path,
        )
        .expect("materialize nested");
    }

    match &value {
        Value::Map(entries) => {
            println!("Map with {} entries", entries.len());
            for (i, (k, v)) in entries.iter().take(5).enumerate() {
                println!("[{i}] key = {k:?}");
                println!("     value = {v:?}");
            }
        }
        Value::Array(items) => {
            println!("Array with {} items", items.len());
            for (i, item) in items.iter().take(5).enumerate() {
                println!("[{i}] = {item:?}");
            }
        }
        other => println!("{other:?}"),
    }
}
