//! Players and Pals, from `worldSaveData.CharacterSaveParameterMap`.
//!
//! Both live in the same map; a player is a character whose `SaveParameter.IsPlayer`
//! is true. Everything else is a Pal, owned by whichever player its
//! `SaveParameter.OwnerPlayerUId` names. That classification is the one this project
//! copies from `oMaN-Rod/palworld-save-pal` (`psp-core/src/domain/world.rs`), which is
//! the actively-maintained reference — see ADR-002.md for why that fork rather than
//! the older Python one.
//!
//! ## Every field is optional, on purpose
//!
//! Palworld renames, retypes and drops `SaveParameter` fields between versions. A
//! character whose `FriendshipPoint` has moved should still render with a level and a
//! species rather than blanking the whole screen, so each field decodes to `Option`
//! and a miss is silent. The parts that *must* work — locating the map, decoding the
//! RawData blob — still error loudly. That split matches how `gvas::value` already
//! treats regions it can't decode: degrade to opaque, never guess.

use crate::gvas::GvasError;
use crate::gvas::nav::{Cursor, guid_to_hex, hex_to_guid};
use crate::rawdata::character;
use crate::rawdata::error::RawDataError;
use crate::world::{self, WorldError};
use std::fmt;

pub const CHARACTER_MAP: &str = "CharacterSaveParameterMap";

#[derive(Debug)]
pub enum CharacterError {
    World(WorldError),
    Gvas(GvasError),
    RawData(RawDataError),
    PlayerNotFound { uid: String },
    MalformedUid { uid: String },
}

impl CharacterError {
    /// Stable machine-readable discriminant for the wasm boundary.
    pub fn code(&self) -> &'static str {
        match self {
            // A save with no character map is the "not a Level.sav" case; keep the
            // world layer's own distinction rather than flattening it.
            CharacterError::World(e) => e.code(),
            CharacterError::Gvas(_) => "gvas_parse_failed",
            CharacterError::RawData(_) => "rawdata_decode_failed",
            CharacterError::PlayerNotFound { .. } => "player_not_found",
            CharacterError::MalformedUid { .. } => "malformed_uid",
        }
    }
}

impl From<WorldError> for CharacterError {
    fn from(e: WorldError) -> Self {
        CharacterError::World(e)
    }
}
impl From<GvasError> for CharacterError {
    fn from(e: GvasError) -> Self {
        CharacterError::Gvas(e)
    }
}
impl From<RawDataError> for CharacterError {
    fn from(e: RawDataError) -> Self {
        CharacterError::RawData(e)
    }
}

impl fmt::Display for CharacterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CharacterError::World(e) => write!(f, "{e}"),
            CharacterError::Gvas(e) => write!(f, "{e}"),
            CharacterError::RawData(e) => write!(f, "{e}"),
            CharacterError::PlayerNotFound { uid } => write!(f, "no player with uid {uid}"),
            CharacterError::MalformedUid { uid } => {
                write!(f, "malformed uid {uid:?}: want 32 hex chars")
            }
        }
    }
}

impl std::error::Error for CharacterError {}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSummary {
    pub uid: String,
    pub instance_id: String,
    pub nickname: Option<String>,
    pub level: Option<i64>,
    pub exp: Option<i64>,
    pub hp: Option<i64>,
    pub shield_hp: Option<i64>,
    pub full_stomach: Option<f32>,
    /// How many Pals name this player as owner. Base-camp Pals have no
    /// `OwnerPlayerUId` and are counted by nobody — in the reference fixture that's
    /// 113 owned of 136 total.
    pub pal_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PalSummary {
    pub instance_id: String,
    pub owner_player_uid: Option<String>,
    /// Species id, e.g. `ChickenPal`. `None` would mean the field moved.
    pub character_id: Option<String>,
    pub nickname: Option<String>,
    pub gender: Option<String>,
    pub level: Option<i64>,
    pub exp: Option<i64>,
    pub hp: Option<i64>,
    /// The three IVs, 0..=100 in game terms.
    pub talent_hp: Option<i64>,
    pub talent_shot: Option<i64>,
    pub talent_defense: Option<i64>,
    pub passive_skills: Vec<String>,
    pub friendship_point: Option<i64>,
    /// Condensation rank. Absent on most Pals (6/136 in the reference fixture).
    pub rank: Option<i64>,
    /// Sanity/SAN. Absent unless it has moved off its default.
    pub sanity_value: Option<f32>,
    /// Rare/boss variant. The field only exists on Pals that are one (4/136 in the
    /// reference fixture), so absence genuinely means "no", not "unknown".
    pub is_rare: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerDetail {
    pub summary: PlayerSummary,
    pub pals: Vec<PalSummary>,
}

/// One decoded character, before it's classified as a player or a Pal.
struct Character {
    /// From the map *key*, not the value.
    player_uid: Option<String>,
    instance_id: String,
    is_player: bool,
    /// The `SaveParameter` property list, plus a cursor rooted in the RawData blob it
    /// was indexed against. The cursor and the blob must travel together — see
    /// `gvas::nav`.
    save_parameter: Vec<crate::gvas::PropertyEntry>,
    blob: Vec<u8>,
    blob_path: String,
    engine_major: u16,
    has_property_guid: bool,
}

impl Character {
    fn cursor(&self) -> Cursor<'_> {
        // Rebuild rather than store: a Cursor borrows the blob, and a struct holding
        // both would be self-referential.
        Cursor::new_raw(
            &self.blob,
            self.engine_major,
            self.has_property_guid,
            &self.blob_path,
        )
    }

    fn text(&self, name: &str) -> Option<String> {
        self.cursor()
            .get_opt(&self.save_parameter, name)
            .and_then(|v| v.as_text())
    }

    fn integer(&self, name: &str) -> Option<i64> {
        self.cursor()
            .get_opt(&self.save_parameter, name)
            .and_then(|v| v.as_integer())
    }

    fn float(&self, name: &str) -> Option<f32> {
        self.cursor()
            .get_opt(&self.save_parameter, name)
            .and_then(|v| v.as_f32())
    }

    fn flag(&self, name: &str) -> bool {
        self.cursor()
            .get_opt(&self.save_parameter, name)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// `Hp` and friends are `StructProperty` wrappers around a single `Value` field
    /// (Palworld's `FFixedPoint64`), not bare integers.
    fn fixed_point(&self, name: &str) -> Option<i64> {
        let cursor = self.cursor();
        let value = cursor.get_opt(&self.save_parameter, name)?;
        let inner = value.as_properties()?;
        cursor
            .get_opt(inner, "Value")
            .and_then(|v| v.as_integer())
            .or_else(|| value.as_integer())
    }

    fn string_array(&self, name: &str) -> Vec<String> {
        self.cursor()
            .get_opt(&self.save_parameter, name)
            .and_then(|v| v.as_array().map(|items| items.to_vec()))
            .map(|items| items.iter().filter_map(|v| v.as_text()).collect())
            .unwrap_or_default()
    }

    fn owner_uid(&self) -> Option<String> {
        self.cursor()
            .get_opt(&self.save_parameter, "OwnerPlayerUId")
            .and_then(|v| v.as_guid())
            .map(|g| guid_to_hex(&g))
    }

    fn to_pal(&self) -> PalSummary {
        PalSummary {
            instance_id: self.instance_id.clone(),
            owner_player_uid: self.owner_uid(),
            character_id: self.text("CharacterID"),
            nickname: self.text("NickName"),
            gender: self
                .text("Gender")
                .map(|g| g.replace("EPalGenderType::", "")),
            level: self.integer("Level"),
            exp: self.integer("Exp"),
            hp: self.fixed_point("Hp"),
            talent_hp: self.integer("Talent_HP"),
            talent_shot: self.integer("Talent_Shot"),
            talent_defense: self.integer("Talent_Defense"),
            passive_skills: self.string_array("PassiveSkillList"),
            friendship_point: self.integer("FriendshipPoint"),
            rank: self.integer("Rank"),
            sanity_value: self.float("SanityValue"),
            is_rare: self.flag("IsRarePal"),
        }
    }

    fn to_player(&self, pal_count: usize) -> PlayerSummary {
        PlayerSummary {
            uid: self.player_uid.clone().unwrap_or_default(),
            instance_id: self.instance_id.clone(),
            nickname: self.text("NickName"),
            level: self.integer("Level"),
            exp: self.integer("Exp"),
            hp: self.fixed_point("Hp"),
            shield_hp: self.fixed_point("ShieldHP"),
            full_stomach: self.float("FullStomach"),
            pal_count,
        }
    }
}

/// Decodes every entry of the character map. One pass; callers filter.
fn load(gvas: &[u8]) -> Result<Vec<Character>, CharacterError> {
    let map = world::open_map(gvas, CHARACTER_MAP)?;
    let value_path = format!("worldSaveData.{CHARACTER_MAP}.Value");

    let mut out = Vec::with_capacity(map.entries.len());
    for entry in &map.entries {
        // The key carries PlayerUId / InstanceId / DebugName. For a Pal, PlayerUId is
        // the all-zero guid, so an empty player_uid here is normal, not a failure.
        let (player_uid, key_instance_id) = match entry.key.as_properties() {
            Some(key_props) => {
                let key_cursor = map.cursor.clone();
                (
                    key_cursor
                        .get_opt(key_props, "PlayerUId")
                        .and_then(|v| v.as_guid())
                        .map(|g| guid_to_hex(&g)),
                    key_cursor
                        .get_opt(key_props, "InstanceId")
                        .and_then(|v| v.as_guid())
                        .map(|g| guid_to_hex(&g)),
                )
            }
            None => (None, None),
        };

        let Some(raw_entry) = crate::gvas::nav::find(&entry.fields, "RawData") else {
            continue;
        };
        let Some(blob) = map
            .cursor
            .materialize(raw_entry)?
            .as_bytes()
            .map(|b| b.to_vec())
        else {
            continue;
        };

        let decoded = character::decode(&blob, map.cursor.has_property_guid())?;
        let blob_path = format!("{value_path}.RawData");

        // SaveParameter's entries are indexed against `blob`, not the save buffer.
        let blob_cursor = map.cursor.rebase(&blob, &blob_path);
        let save_parameter = match blob_cursor.get_opt(&decoded.object, "SaveParameter") {
            Some(v) => v.as_properties().map(|p| p.to_vec()).unwrap_or_default(),
            None => Vec::new(),
        };
        let is_player = blob_cursor
            .get_opt(&save_parameter, "IsPlayer")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        out.push(Character {
            player_uid: player_uid.filter(|u| u.chars().any(|c| c != '0')),
            instance_id: key_instance_id.unwrap_or_default(),
            is_player,
            save_parameter,
            blob,
            blob_path,
            engine_major: map.cursor.engine_major(),
            has_property_guid: map.cursor.has_property_guid(),
        });
    }
    Ok(out)
}

fn pal_count_for(characters: &[Character], uid: &str) -> usize {
    characters
        .iter()
        .filter(|c| !c.is_player && c.owner_uid().as_deref() == Some(uid))
        .count()
}

/// Every player in the save. Size is proportional to the player count, never to the
/// size of the save.
pub fn list_players(gvas: &[u8]) -> Result<Vec<PlayerSummary>, CharacterError> {
    let characters = load(gvas)?;
    Ok(characters
        .iter()
        .filter(|c| c.is_player)
        .map(|c| {
            let count = c
                .player_uid
                .as_deref()
                .map(|uid| pal_count_for(&characters, uid))
                .unwrap_or(0);
            c.to_player(count)
        })
        .collect())
}

/// Every Pal owned by `owner_uid`.
pub fn pals_of(gvas: &[u8], owner_uid: &str) -> Result<Vec<PalSummary>, CharacterError> {
    if hex_to_guid(owner_uid).is_none() {
        return Err(CharacterError::MalformedUid {
            uid: owner_uid.to_string(),
        });
    }
    let characters = load(gvas)?;
    Ok(characters
        .iter()
        .filter(|c| !c.is_player && c.owner_uid().as_deref() == Some(owner_uid))
        .map(Character::to_pal)
        .collect())
}

/// One player plus their Pals, in a single pass over the map.
pub fn player(gvas: &[u8], uid: &str) -> Result<PlayerDetail, CharacterError> {
    if hex_to_guid(uid).is_none() {
        return Err(CharacterError::MalformedUid {
            uid: uid.to_string(),
        });
    }
    let characters = load(gvas)?;

    let pals: Vec<PalSummary> = characters
        .iter()
        .filter(|c| !c.is_player && c.owner_uid().as_deref() == Some(uid))
        .map(Character::to_pal)
        .collect();

    let found = characters
        .iter()
        .find(|c| c.is_player && c.player_uid.as_deref() == Some(uid))
        .ok_or_else(|| CharacterError::PlayerNotFound {
            uid: uid.to_string(),
        })?;

    Ok(PlayerDetail {
        summary: found.to_player(pals.len()),
        pals,
    })
}

/// Every Pal in the save, regardless of owner — including any whose owner no longer
/// exists, which is exactly the orphaned state a future repair tool would target.
pub fn list_all_pals(gvas: &[u8]) -> Result<Vec<PalSummary>, CharacterError> {
    let characters = load(gvas)?;
    Ok(characters
        .iter()
        .filter(|c| !c.is_player)
        .map(Character::to_pal)
        .collect())
}
