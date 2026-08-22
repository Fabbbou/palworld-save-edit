//! Dev tool: materializes and walks a GVAS file's property tree a few levels deep,
//! to sanity-check `materialize()` against real fixtures (Struct(Properties) nesting,
//! Map key/value shapes, RawData byte blobs).
//!
//! Usage: cargo run --example explore -p palsave -- <path-to-.sav>

use palsave::gvas::value::{StructValue, Value};
use palsave::gvas::{GvasFile, PropertyEntry};

fn describe(value: &Value, depth: usize, max_depth: usize) -> String {
    let indent = "  ".repeat(depth);
    match value {
        Value::Int(v) => format!("Int({v})"),
        Value::UInt16(v) => format!("UInt16({v})"),
        Value::UInt32(v) => format!("UInt32({v})"),
        Value::Int64(v) => format!("Int64({v})"),
        Value::Float(v) => format!("Float({v})"),
        Value::Str(s) => format!("Str({:?})", s.display_lossy()),
        Value::Name(s) => format!("Name({:?})", s.display_lossy()),
        Value::Bool(v) => format!("Bool({v})"),
        Value::Byte(v) => format!("Byte({v})"),
        Value::ByteLabel(s) => format!("ByteLabel({:?})", s.display_lossy()),
        Value::Enum(s) => format!("Enum({:?})", s.display_lossy()),
        Value::Bytes(b) => format!("Bytes[{} bytes]", b.len()),
        Value::Raw(b) => format!("Raw[{} bytes, could not decode]", b.len()),
        Value::Array(items) => {
            if depth >= max_depth {
                format!("Array[{} items]", items.len())
            } else {
                let mut s = format!("Array[{} items]", items.len());
                for (i, item) in items.iter().take(3).enumerate() {
                    s.push_str(&format!(
                        "\n{indent}  [{i}] {}",
                        describe(item, depth + 1, max_depth)
                    ));
                }
                if items.len() > 3 {
                    s.push_str(&format!("\n{indent}  ... {} more", items.len() - 3));
                }
                s
            }
        }
        Value::Map(entries) => {
            if depth >= max_depth {
                format!("Map[{} entries]", entries.len())
            } else {
                let mut s = format!("Map[{} entries]", entries.len());
                for (i, (k, v)) in entries.iter().take(3).enumerate() {
                    s.push_str(&format!(
                        "\n{indent}  [{i}] key={} value={}",
                        describe(k, depth + 1, max_depth),
                        describe(v, depth + 1, max_depth)
                    ));
                }
                if entries.len() > 3 {
                    s.push_str(&format!("\n{indent}  ... {} more", entries.len() - 3));
                }
                s
            }
        }
        Value::Struct(StructValue::Guid(g)) => format!("Guid({g:02x?})"),
        Value::Struct(StructValue::DateTime(t)) => format!("DateTime(ticks={t})"),
        Value::Struct(StructValue::Vector { x, y, z }) => format!("Vector({x}, {y}, {z})"),
        Value::Struct(StructValue::Quat { x, y, z, w }) => format!("Quat({x}, {y}, {z}, {w})"),
        Value::Struct(StructValue::LinearColor { r, g, b, a }) => {
            format!("LinearColor({r}, {g}, {b}, {a})")
        }
        Value::Struct(StructValue::Properties(props)) => {
            describe_properties(props, depth, max_depth)
        }
    }
}

fn describe_properties(props: &[PropertyEntry], depth: usize, max_depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut s = format!("Struct{{{} properties}}", props.len());
    if depth >= max_depth {
        return s;
    }
    for p in props {
        s.push_str(&format!("\n{indent}  {} : {}", p.name, p.type_name));
    }
    s
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: explore <path-to-.sav>");
        std::process::exit(1);
    });

    let bytes = std::fs::read(&path).expect("read");
    let container = palsave::container::decode(&bytes).expect("container decode");
    let file = GvasFile::parse(&container.gvas).expect("gvas parse");

    println!("{path}: {} top-level properties\n", file.properties.len());
    for i in 0..file.properties.len() {
        let entry = &file.properties[i];
        let value = file
            .materialize(i)
            .expect("materialize never hard-fails, falls back to Raw");
        println!(
            "{} : {}\n  = {}\n",
            entry.name,
            entry.type_name,
            describe(&value, 1, 3)
        );
    }
}
