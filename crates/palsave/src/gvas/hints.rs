//! Path -> struct-type hints for `MapProperty` keys/values whose real struct type
//! isn't recorded on the wire (see the `TagExtra::Map` doc comment in `value.rs` for
//! why that lookup is needed at all). Ported verbatim from `oMaN-Rod/uesave-rs`
//! (branch `pluggable-game-support`, MIT), `games/palworld/mod.rs`'s
//! `palworld_types()` — the same actively-maintained fork ADR-002.md credits for the
//! current `GroupSaveDataMap` layout. Paths are dotted from the save root exactly as
//! `GvasFile::materialize` builds them (e.g. `worldSaveData.GroupSaveDataMap.Key`).
//!
//! Two lists: `STRUCT_HINTS` are paths whose key or value is a *generic* struct (a
//! plain None-terminated property list — `StructType::Struct(None)` upstream, our
//! `StructValue::Properties`); `GUID_HINTS` are paths whose key or value is a bare
//! 16-byte Guid. A path in neither list falls back to the "Guid for keys, generic
//! struct for values" default from `TagExtra::Map`'s doc comment — the same default
//! uesave-rs itself uses when its own hint table (which this list *is*, ported) comes
//! up empty.

const STRUCT_HINTS: &[&str] = &[
    "worldSaveData.CharacterContainerSaveData.Key",
    "worldSaveData.CharacterSaveParameterMap.Key",
    "worldSaveData.CharacterSaveParameterMap.Value",
    "worldSaveData.FoliageGridSaveDataMap.Key",
    "worldSaveData.FoliageGridSaveDataMap.Value",
    "worldSaveData.FoliageGridSaveDataMap.ModelMap.Value",
    "worldSaveData.FoliageGridSaveDataMap.ModelMap.InstanceDataMap.Key",
    "worldSaveData.FoliageGridSaveDataMap.ModelMap.InstanceDataMap.Value",
    "worldSaveData.ItemContainerSaveData.Key",
    "worldSaveData.ItemContainerSaveData.Value",
    "worldSaveData.MapObjectSaveData.ConcreteModel.ModuleMap.Value",
    "worldSaveData.MapObjectSaveData.Model.EffectMap.Value",
    "worldSaveData.MapObjectSpawnerInStageSaveData.Key",
    "worldSaveData.MapObjectSpawnerInStageSaveData.Value",
    "worldSaveData.MapObjectSpawnerInStageSaveData.Value.SpawnerDataMapByLevelObjectInstanceId.Value",
    "worldSaveData.MapObjectSpawnerInStageSaveData.Value.SpawnerDataMapByLevelObjectInstanceId.Value.ItemMap.Value",
    "worldSaveData.WorkSaveData.WorkAssignMap.Value",
    "worldSaveData.BaseCampSaveData.Value",
    "worldSaveData.BaseCampSaveData.ModuleMap.Value",
    "worldSaveData.CharacterContainerSaveData.Value",
    "worldSaveData.GroupSaveDataMap.Value",
    "worldSaveData.EnemyCampSaveData.EnemyCampStatusMap.Value",
    "worldSaveData.EnemyCampSaveData.EnemyCampStatusMap.Value.TreasureBoxInfoMapBySpawnerName.Value",
    "worldSaveData.DungeonSaveData.MapObjectSaveData.Model.EffectMap.Value",
    "worldSaveData.DungeonSaveData.MapObjectSaveData.ConcreteModel.ModuleMap.Value",
    "worldSaveData.InvaderSaveData.Value",
    "worldSaveData.OilrigSaveData.OilrigMap.Value",
    "worldSaveData.SupplySaveData.SupplyInfos.Value",
    "worldSaveData.GuildExtraSaveDataMap.Value",
    "SaveData.Local_MaxFriendshipPalIds.Value",
    "SaveData.Local_MaxFriendshipPalIds.Key",
    "worldSaveData.MapObjectSpawnerInStageSaveData.SpawnerDataMapByLevelObjectInstanceId.Value",
    "worldSaveData.MapObjectSpawnerInStageSaveData.SpawnerDataMapByLevelObjectInstanceId.ItemMap.Value",
    "worldSaveData.DungeonSaveData.RewardSaveDataMap.Value",
];

const GUID_HINTS: &[&str] = &[
    "worldSaveData.MapObjectSpawnerInStageSaveData.Value.SpawnerDataMapByLevelObjectInstanceId.Key",
    "worldSaveData.BaseCampSaveData.Key",
    "worldSaveData.GroupSaveDataMap.Key",
    "worldSaveData.InvaderSaveData.Key",
    "worldSaveData.SupplySaveData.SupplyInfos.Key",
    "worldSaveData.GuildExtraSaveDataMap.Key",
    "worldSaveData.MapObjectSpawnerInStageSaveData.SpawnerDataMapByLevelObjectInstanceId.Key",
    "worldSaveData.DungeonSaveData.RewardSaveDataMap.Key",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructHint {
    Guid,
    Generic,
}

/// Looks up `path` (already suffixed with `.Key` or `.Value` by the caller). `None`
/// means "no hint, use the default" — see the module docs.
pub fn lookup(path: &str) -> Option<StructHint> {
    if STRUCT_HINTS.contains(&path) {
        Some(StructHint::Generic)
    } else if GUID_HINTS.contains(&path) {
        Some(StructHint::Guid)
    } else {
        None
    }
}
