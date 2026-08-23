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

use crate::edit::{self, Scalar};
use crate::gvas::GvasError;
use crate::gvas::nav::{Cursor, guid_to_hex, hex_to_guid};
use crate::gvas::property::read_property_tag;
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
    PlayerNotFound {
        uid: String,
    },
    MalformedUid {
        uid: String,
    },
    Edit(crate::edit::EditError),
    /// The character exists but has no such property. Palworld omits a field until it
    /// differs from its default, so this is common — and it is refused rather than
    /// inserted, because adding a property changes the list length, which is the
    /// map-entry-insert problem `edit/mod.rs` documents as unsupported.
    FieldNotPresent {
        field: &'static str,
    },
    /// A value outside what the game will accept. Refused, not clamped: a silently
    /// corrected edit leaves the user unable to tell which one was wrong.
    OutOfRange {
        field: &'static str,
        value: i64,
        min: i64,
        max: i64,
    },
    /// The character's blob no longer decodes after the edit. The buffer is discarded.
    BlobVerificationFailed,
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
            CharacterError::Edit(_) => "edit_failed",
            CharacterError::FieldNotPresent { .. } => "field_not_present",
            CharacterError::OutOfRange { .. } => "value_out_of_range",
            CharacterError::BlobVerificationFailed => "blob_verification_failed",
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
impl From<crate::edit::EditError> for CharacterError {
    fn from(e: crate::edit::EditError) -> Self {
        CharacterError::Edit(e)
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
            CharacterError::Edit(e) => write!(f, "{e}"),
            CharacterError::FieldNotPresent { field } => {
                write!(f, "this character has no {field} field to edit")
            }
            CharacterError::OutOfRange {
                field,
                value,
                min,
                max,
            } => write!(f, "{field} must be between {min} and {max}, got {value}"),
            CharacterError::BlobVerificationFailed => {
                write!(f, "the edited character data no longer decodes")
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
    /// The pieces an edit needs, which reading alone does not.
    ///
    /// Splicing a stat is two nested operations: patch inside `blob` using a
    /// blob-relative chain `[save_parameter_entry, <stat>]`, then swap the whole
    /// re-encoded blob into the save using the save-relative chain
    /// `[world_entry, map_entry, raw_entry]`. Reading discards all four of these, so
    /// they are kept here rather than re-walking the 8.5 MB map to recover them.
    world_entry: crate::gvas::PropertyEntry,
    map_entry: crate::gvas::PropertyEntry,
    /// `RawData`, save-relative.
    raw_entry: crate::gvas::PropertyEntry,
    /// `SaveParameter`, blob-relative — the entry itself, not just its children.
    save_parameter_entry: Option<crate::gvas::PropertyEntry>,
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
        let save_parameter_entry =
            crate::gvas::nav::find(&decoded.object, "SaveParameter").cloned();

        out.push(Character {
            player_uid: player_uid.filter(|u| u.chars().any(|c| c != '0')),
            instance_id: key_instance_id.unwrap_or_default(),
            is_player,
            save_parameter,
            blob,
            blob_path,
            engine_major: map.cursor.engine_major(),
            has_property_guid: map.cursor.has_property_guid(),
            world_entry: map.world_entry.clone(),
            map_entry: map.map_entry.clone(),
            raw_entry: raw_entry.clone(),
            save_parameter_entry,
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

// ---------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------

/// Editable Pal stats. A curated set, not an arbitrary path API: nothing would stop a
/// caller writing nonsense into a field the game depends on, and the ranges below are
/// only meaningful because the set is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalStat {
    Level,
    Exp,
    TalentHp,
    TalentShot,
    TalentDefense,
}

/// Editable player stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerStat {
    Level,
    Exp,
}

/// `(property name, min, max)`. IVs are a 0..=100 game stat; level is capped well
/// above the current game maximum so a future level-cap bump doesn't make this
/// refuse valid saves, while still rejecting obvious garbage.
const fn pal_stat_spec(stat: PalStat) -> (&'static str, i64, i64) {
    match stat {
        PalStat::Level => ("Level", 1, 255),
        PalStat::Exp => ("Exp", 0, i64::MAX),
        PalStat::TalentHp => ("Talent_HP", 0, 100),
        PalStat::TalentShot => ("Talent_Shot", 0, 100),
        PalStat::TalentDefense => ("Talent_Defense", 0, 100),
    }
}

const fn player_stat_spec(stat: PlayerStat) -> (&'static str, i64, i64) {
    match stat {
        PlayerStat::Level => ("Level", 1, 255),
        PlayerStat::Exp => ("Exp", 0, i64::MAX),
    }
}

impl Character {
    /// Rewrites one property inside this character's `RawData` blob and splices the
    /// result back into the save.
    ///
    /// Two nested applications of the same engine, inner first. The blob's property
    /// spans are blob-relative, so the inner chain must be spliced against `blob`;
    /// only once the blob is whole again can it be swapped into the save. See
    /// `gvas::nav` for why mixing the two buffers is the trap this guards against.
    fn write_property(
        &self,
        level_gvas: &[u8],
        field: &'static str,
        value: &Scalar,
    ) -> Result<Vec<u8>, CharacterError> {
        let leaf = crate::gvas::nav::find(&self.save_parameter, field)
            .ok_or(CharacterError::FieldNotPresent { field })?;
        let save_parameter_entry =
            self.save_parameter_entry
                .as_ref()
                .ok_or(CharacterError::FieldNotPresent {
                    field: "SaveParameter",
                })?;

        // Encode against the type declared on disk, never against the caller's guess.
        let mut pos = leaf.span.start;
        let tag = read_property_tag(&self.blob, &mut pos, self.has_property_guid)?
            .ok_or(CharacterError::FieldNotPresent { field })?;
        let new_value = edit::encode_scalar(&tag, value)?;

        // Inner: patch within the blob, fixing SaveParameter's size field.
        let new_blob = edit::replace_property_value(
            &self.blob,
            &[save_parameter_entry, leaf],
            new_value,
            self.has_property_guid,
        )?
        .apply(&self.blob)?;

        // `verify_reparses` needs a GVAS header and a blob has none, so the blob-level
        // check is that it still decodes and consumes exactly — which is what catches
        // a botched size fixup.
        character::decode(&new_blob, self.has_property_guid)
            .map_err(|_| CharacterError::BlobVerificationFailed)?;

        // Outer: swap the whole blob into the save, fixing the enclosing sizes.
        let edited = edit::replace_property_value(
            level_gvas,
            &[&self.world_entry, &self.map_entry, &self.raw_entry],
            edit::byte_array_value(&new_blob),
            self.has_property_guid,
        )?
        .apply(level_gvas)?;

        edit::verify_reparses(&edited)?;
        Ok(edited)
    }
}

fn check_range(field: &'static str, value: i64, min: i64, max: i64) -> Result<(), CharacterError> {
    if value < min || value > max {
        return Err(CharacterError::OutOfRange {
            field,
            value,
            min,
            max,
        });
    }
    Ok(())
}

/// Widens an integer to whatever the property's declared type needs. `Level` is a
/// `ByteProperty` while `Exp` is an `Int64Property`, so the caller passing a plain
/// `i64` is turned into the right shape here rather than at every call site.
fn scalar_for(type_name: &str, value: i64) -> Option<Scalar> {
    match type_name {
        "ByteProperty" => u8::try_from(value).ok().map(Scalar::Byte),
        "IntProperty" => i32::try_from(value).ok().map(Scalar::Int),
        "Int64Property" => Some(Scalar::Int64(value)),
        _ => None,
    }
}

fn set_stat_on(
    level_gvas: &[u8],
    character: &Character,
    field: &'static str,
    value: i64,
) -> Result<Vec<u8>, CharacterError> {
    let leaf = crate::gvas::nav::find(&character.save_parameter, field)
        .ok_or(CharacterError::FieldNotPresent { field })?;
    let scalar = scalar_for(&leaf.type_name, value).ok_or(CharacterError::OutOfRange {
        field,
        value,
        min: 0,
        max: i64::from(u8::MAX),
    })?;
    character.write_property(level_gvas, field, &scalar)
}

/// Sets one stat on the Pal with `instance_id`, returning a fresh GVAS buffer.
pub fn set_pal_stat(
    level_gvas: &[u8],
    instance_id: &str,
    stat: PalStat,
    value: i64,
) -> Result<Vec<u8>, CharacterError> {
    let (field, min, max) = pal_stat_spec(stat);
    check_range(field, value, min, max)?;

    let characters = load(level_gvas)?;
    let target = characters
        .iter()
        .find(|c| !c.is_player && c.instance_id == instance_id)
        .ok_or_else(|| CharacterError::PlayerNotFound {
            uid: instance_id.to_string(),
        })?;
    set_stat_on(level_gvas, target, field, value)
}

/// Renames the Pal with `instance_id`.
pub fn set_pal_nickname(
    level_gvas: &[u8],
    instance_id: &str,
    nickname: &str,
) -> Result<Vec<u8>, CharacterError> {
    let characters = load(level_gvas)?;
    let target = characters
        .iter()
        .find(|c| !c.is_player && c.instance_id == instance_id)
        .ok_or_else(|| CharacterError::PlayerNotFound {
            uid: instance_id.to_string(),
        })?;
    target.write_property(level_gvas, "NickName", &Scalar::Text(nickname.to_string()))
}

/// Sets one stat on the player with `uid`.
pub fn set_player_stat(
    level_gvas: &[u8],
    uid: &str,
    stat: PlayerStat,
    value: i64,
) -> Result<Vec<u8>, CharacterError> {
    if hex_to_guid(uid).is_none() {
        return Err(CharacterError::MalformedUid {
            uid: uid.to_string(),
        });
    }
    let (field, min, max) = player_stat_spec(stat);
    check_range(field, value, min, max)?;

    let characters = load(level_gvas)?;
    let target = characters
        .iter()
        .find(|c| c.is_player && c.player_uid.as_deref() == Some(uid))
        .ok_or_else(|| CharacterError::PlayerNotFound {
            uid: uid.to_string(),
        })?;
    set_stat_on(level_gvas, target, field, value)
}
