//! Dev tool: decodes one CharacterSaveParameterMap entry's RawData (a player or Pal)
//! and prints its top-level stat property names/types, to sanity-check
//! rawdata::character against real data.
use palsave::gvas::value::{StructValue, Value};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let which: usize = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(0);
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();
    let has_property_guid = file.header.has_property_guid();

    let idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(world_props)) = file.materialize(idx).unwrap() else {
        panic!()
    };
    let map_entry = world_props
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .unwrap();
    let map = palsave::gvas::value::materialize_property(
        &container.gvas,
        map_entry,
        file.header.engine_version_major,
        has_property_guid,
        "worldSaveData.CharacterSaveParameterMap",
    )
    .unwrap();
    let Value::Map(entries) = map else { panic!() };

    let (key, value) = &entries[which];
    println!("key = {key:?}\n");
    let Value::Struct(StructValue::Properties(fields)) = value else {
        panic!()
    };
    let rd_entry = fields.iter().find(|f| f.name == "RawData").unwrap();
    let rd = palsave::gvas::value::materialize_property(
        &container.gvas,
        rd_entry,
        file.header.engine_version_major,
        has_property_guid,
        "worldSaveData.CharacterSaveParameterMap.Value.RawData",
    )
    .unwrap();
    let Value::Bytes(rd) = rd else { panic!() };

    let decoded = palsave::rawdata::character::decode(&rd, has_property_guid).unwrap();
    println!("group_id = {:02x?}", decoded.group_id);
    println!("unknown_bytes = {:02x?}", decoded.unknown_bytes);
    println!("trailing_bytes = {:02x?}", decoded.trailing_bytes);
    println!("object ({} properties):", decoded.object.len());
    for p in &decoded.object {
        print!("  {} : {}", p.name, p.type_name);
        if matches!(
            p.type_name.as_str(),
            "IntProperty" | "FloatProperty" | "StrProperty" | "NameProperty" | "BoolProperty"
        ) {
            let v = palsave::gvas::value::materialize_property(
                &rd,
                p,
                file.header.engine_version_major,
                has_property_guid,
                &p.name,
            )
            .unwrap();
            print!(" = {v:?}");
        }
        println!();
        if p.name == "SaveParameter" {
            let v = palsave::gvas::value::materialize_property(
                &rd,
                p,
                file.header.engine_version_major,
                has_property_guid,
                "SaveParameter",
            )
            .unwrap();
            if let Value::Struct(StructValue::Properties(inner)) = v {
                for ip in &inner {
                    print!("    {} : {}", ip.name, ip.type_name);
                    if matches!(
                        ip.type_name.as_str(),
                        "IntProperty"
                            | "FloatProperty"
                            | "StrProperty"
                            | "NameProperty"
                            | "BoolProperty"
                            | "EnumProperty"
                    ) {
                        let iv = palsave::gvas::value::materialize_property(
                            &rd,
                            ip,
                            file.header.engine_version_major,
                            has_property_guid,
                            &format!("SaveParameter.{}", ip.name),
                        )
                        .unwrap();
                        print!(" = {iv:?}");
                    }
                    println!();
                }
            }
        }
    }
}
