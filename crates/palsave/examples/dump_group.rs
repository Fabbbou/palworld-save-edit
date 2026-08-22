//! Dev tool: dumps every GroupSaveDataMap RawData blob's hex bytes + GroupType, to
//! debug the group.rs decoder against real data.
use palsave::gvas::value::{StructValue, Value};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let container = palsave::container::decode(&bytes).unwrap();
    let file = palsave::gvas::GvasFile::parse(&container.gvas).unwrap();

    let idx = file
        .properties
        .iter()
        .position(|p| p.name == "worldSaveData")
        .unwrap();
    let Value::Struct(StructValue::Properties(world_props)) = file.materialize(idx).unwrap() else {
        panic!()
    };
    let group_map_entry = world_props
        .iter()
        .find(|p| p.name == "GroupSaveDataMap")
        .unwrap();
    let group_map = palsave::gvas::value::materialize_property(
        &container.gvas,
        group_map_entry,
        file.header.engine_version_major,
        file.header.has_property_guid(),
        "worldSaveData.GroupSaveDataMap",
    )
    .unwrap();
    let Value::Map(entries) = group_map else {
        panic!()
    };

    for (i, (_k, v)) in entries.iter().enumerate() {
        let Value::Struct(StructValue::Properties(fields)) = v else {
            panic!()
        };
        let gt_entry = fields.iter().find(|f| f.name == "GroupType").unwrap();
        let gt = palsave::gvas::value::materialize_property(
            &container.gvas,
            gt_entry,
            file.header.engine_version_major,
            file.header.has_property_guid(),
            "worldSaveData.GroupSaveDataMap.Value.GroupType",
        )
        .unwrap();
        let Value::Enum(gt) = gt else { panic!() };
        let rd_entry = fields.iter().find(|f| f.name == "RawData").unwrap();
        let rd = palsave::gvas::value::materialize_property(
            &container.gvas,
            rd_entry,
            file.header.engine_version_major,
            file.header.has_property_guid(),
            "worldSaveData.GroupSaveDataMap.Value.RawData",
        )
        .unwrap();
        let Value::Bytes(rd) = rd else { panic!() };
        let gt_str = gt.display_lossy();
        match palsave::rawdata::group::decode(&rd, &gt_str) {
            Ok(decoded) => {
                let re = palsave::rawdata::group::encode(&decoded);
                let ok = re == rd;
                println!(
                    "[{i}] group_type={gt_str} raw_data_len={} decode=OK round_trip={ok}",
                    rd.len()
                );
            }
            Err(e) => {
                println!(
                    "[{i}] group_type={gt_str} raw_data_len={} decode=FAIL: {e}",
                    rd.len()
                );
            }
        }
    }
}
