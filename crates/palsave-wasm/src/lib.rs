//! wasm-bindgen bindings for `palsave`. **Bindings only — no logic.** Every function
//! here is a shim: convert arguments, call one `palsave` function, convert the result.
//! Anything that needs a decision belongs in `palsave`, where it stays testable with
//! plain `cargo test` and no browser (see `CLAUDE.md`).
//!
//! ## The boundary rule
//!
//! The save never crosses into JavaScript. [`SaveHandle`] owns the decompressed GVAS
//! buffer in wasm linear memory; JS holds an opaque handle. Reads return small view
//! models sized by *what was asked for* (a guild list, one guild's roster), never by
//! the size of the save. Writes are commands sent in. The single exception is
//! [`SaveHandle::export`], which is called once and by definition returns the whole
//! file.
//!
//! Exactly one copy of the decompressed buffer is held at a time. An edit replaces it
//! with the spliced result rather than keeping both — wasm32 has a 4 GB address-space
//! ceiling and `Level.sav` decompresses to ~8.5 MB, so a handle that accumulated
//! copies per edit would be a real problem on a big server save.

use palsave::characters;
use palsave::container::{self, Algorithm, Container};
use palsave::guilds;
use palsave::inventory;
use serde::Serialize;
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

/// Typed error carrying a stable machine-readable `code` alongside the human message.
/// Callers branch on `code`; `message` is for display and diagnostics only.
#[derive(Serialize)]
struct ErrorPayload {
    code: &'static str,
    message: String,
}

fn js_error(code: &'static str, message: impl std::fmt::Display) -> JsValue {
    let payload = ErrorPayload {
        code,
        message: message.to_string(),
    };
    serde_wasm_bindgen::to_value(&payload).unwrap_or_else(|_| JsValue::from_str(code))
}

fn guild_error(e: guilds::GuildError) -> JsValue {
    js_error(e.code(), e)
}

fn character_error(e: characters::CharacterError) -> JsValue {
    js_error(e.code(), e)
}

fn inventory_error(e: inventory::InventoryError) -> JsValue {
    js_error(e.code(), e)
}

/// Stat names cross as strings. An unknown one is refused rather than defaulted —
/// a typo silently editing the wrong stat is exactly the kind of quiet wrongness
/// this project refuses elsewhere.
fn parse_pal_stat(name: &str) -> Result<characters::PalStat, JsValue> {
    match name {
        "level" => Ok(characters::PalStat::Level),
        "exp" => Ok(characters::PalStat::Exp),
        "talent_hp" => Ok(characters::PalStat::TalentHp),
        "talent_shot" => Ok(characters::PalStat::TalentShot),
        "talent_defense" => Ok(characters::PalStat::TalentDefense),
        other => Err(js_error(
            "unknown_stat",
            format!("unknown pal stat {other:?}"),
        )),
    }
}

fn parse_player_stat(name: &str) -> Result<characters::PlayerStat, JsValue> {
    match name {
        "level" => Ok(characters::PlayerStat::Level),
        "exp" => Ok(characters::PlayerStat::Exp),
        other => Err(js_error(
            "unknown_stat",
            format!("unknown player stat {other:?}"),
        )),
    }
}

/// `serde_wasm_bindgen` failures mean a view model didn't serialize — a bug here, not
/// bad input from the caller, so it gets its own code rather than being conflated
/// with a parse or edit failure.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    serde_wasm_bindgen::to_value(value).map_err(|e| js_error("serialization_failed", e))
}

#[derive(Serialize)]
struct Summary {
    save_game_type: String,
    engine_version: String,
    save_game_version: u32,
    /// Decompressed GVAS size in bytes. A number, not the bytes.
    gvas_len: usize,
    top_level_property_count: usize,
    container: ContainerInfo,
}

#[derive(Serialize)]
struct ContainerInfo {
    /// `"PlZ"` (zlib) or `"PlM"` (Oodle Mermaid), as found on open.
    format: &'static str,
    was_cnk_wrapped: bool,
    /// True when `export()` will write a different container format than the one this
    /// save was opened from. No open-source Oodle *compressor* exists, so a PlM save
    /// is written back as PlZ: larger, and not byte-identical. The game reads both.
    /// The UI is expected to surface this before an in-place write.
    will_downgrade_to_zlib: bool,
}

#[derive(Serialize)]
struct GuildSummaryView {
    id: String,
    group_type: String,
    name: String,
    member_count: usize,
    base_camp_level: i32,
    pal_count: usize,
}

impl From<&guilds::GuildSummary> for GuildSummaryView {
    fn from(s: &guilds::GuildSummary) -> Self {
        GuildSummaryView {
            id: s.id.clone(),
            group_type: s.group_type.clone(),
            name: s.name.clone(),
            member_count: s.member_count,
            base_camp_level: s.base_camp_level,
            pal_count: s.pal_count,
        }
    }
}

#[derive(Serialize)]
struct GuildMemberView {
    player_uid: String,
    player_name: String,
    /// Unreal `FDateTime` ticks. Serialized as a string because this exceeds 2^53 in
    /// practice and would lose precision as a JS `number` — the exact class of
    /// silent corruption `CLAUDE.md` picks Rust to avoid.
    last_online_real_time: String,
    role: Option<u8>,
}

#[derive(Serialize)]
struct GuildDetailView {
    summary: GuildSummaryView,
    admin_player_uid: Option<String>,
    members: Vec<GuildMemberView>,
}

/// `i64` game stats cross as strings for the same reason `GuildMemberView` does:
/// exp and fixed-point HP both run past what a JS number holds exactly, and a
/// silently-rounded stat is worse than no stat.
#[derive(Serialize)]
struct PlayerSummaryView {
    uid: String,
    instance_id: String,
    nickname: Option<String>,
    level: Option<i64>,
    exp: Option<String>,
    hp: Option<String>,
    shield_hp: Option<String>,
    full_stomach: Option<f32>,
    pal_count: usize,
}

impl From<&characters::PlayerSummary> for PlayerSummaryView {
    fn from(p: &characters::PlayerSummary) -> Self {
        PlayerSummaryView {
            uid: p.uid.clone(),
            instance_id: p.instance_id.clone(),
            nickname: p.nickname.clone(),
            level: p.level,
            exp: p.exp.map(|v| v.to_string()),
            hp: p.hp.map(|v| v.to_string()),
            shield_hp: p.shield_hp.map(|v| v.to_string()),
            full_stomach: p.full_stomach,
            pal_count: p.pal_count,
        }
    }
}

#[derive(Serialize)]
struct PalSummaryView {
    instance_id: String,
    owner_player_uid: Option<String>,
    character_id: Option<String>,
    nickname: Option<String>,
    gender: Option<String>,
    level: Option<i64>,
    exp: Option<String>,
    hp: Option<String>,
    talent_hp: Option<i64>,
    talent_shot: Option<i64>,
    talent_defense: Option<i64>,
    passive_skills: Vec<String>,
    friendship_point: Option<i64>,
    rank: Option<i64>,
    sanity_value: Option<f32>,
    is_rare: bool,
}

impl From<&characters::PalSummary> for PalSummaryView {
    fn from(p: &characters::PalSummary) -> Self {
        PalSummaryView {
            instance_id: p.instance_id.clone(),
            owner_player_uid: p.owner_player_uid.clone(),
            character_id: p.character_id.clone(),
            nickname: p.nickname.clone(),
            gender: p.gender.clone(),
            level: p.level,
            exp: p.exp.map(|v| v.to_string()),
            hp: p.hp.map(|v| v.to_string()),
            talent_hp: p.talent_hp,
            talent_shot: p.talent_shot,
            talent_defense: p.talent_defense,
            passive_skills: p.passive_skills.clone(),
            friendship_point: p.friendship_point,
            rank: p.rank,
            sanity_value: p.sanity_value,
            is_rare: p.is_rare,
        }
    }
}

#[derive(Serialize)]
struct PlayerDetailView {
    summary: PlayerSummaryView,
    pals: Vec<PalSummaryView>,
}

#[derive(Serialize)]
struct SlotView {
    slot_index: i32,
    count: i32,
    static_id: Option<String>,
    /// From the slot's `DynamicItemSaveData` row. Absent for most items — a stack of
    /// Wood has no per-instance state — so the UI must read `null` as "not applicable"
    /// rather than "unknown".
    durability: Option<f32>,
    remaining_bullets: Option<i32>,
    ammo_static_id: Option<String>,
    egg_character_id: Option<String>,
}

#[derive(Serialize)]
struct ContainerView {
    kind: &'static str,
    id: String,
    slot_count: i32,
    slots: Vec<SlotView>,
    missing: bool,
}

#[derive(Serialize)]
struct PlayerInventoryView {
    player_uid: String,
    containers: Vec<ContainerView>,
}

/// One Pal-box or party slot. `pal` reuses [`PalSummaryView`] rather than defining a
/// slimmer shape, because it is literally the same Pal the Players screen shows — the
/// join goes through `characters::list_all_pals`, so the two screens read one decoder.
#[derive(Serialize)]
struct PalSlotView {
    slot_index: i32,
    instance_id: String,
    pal: Option<PalSummaryView>,
}

#[derive(Serialize)]
struct PalContainerView {
    kind: &'static str,
    id: String,
    slot_count: i32,
    slots: Vec<PalSlotView>,
    missing: bool,
}

#[derive(Serialize)]
struct PlayerPalStorageView {
    player_uid: String,
    containers: Vec<PalContainerView>,
}

/// A report a user can attach to a bug report. Deliberately carries **no** personal
/// data: no player names, no uids, no guild names, no item ids — only format
/// structure and counts. That claim is enforced by a test, not by this comment
/// (`diagnostic_report_carries_no_personal_data`).
#[derive(Serialize)]
struct DiagnosticReport {
    engine_version: String,
    save_game_version: u32,
    package_version_ue4: u32,
    package_version_ue5: Option<u32>,
    save_game_class: String,
    container_format: &'static str,
    was_cnk_wrapped: bool,
    will_downgrade_to_zlib: bool,
    gvas_len: usize,
    /// Property *names* — format structure, not user content.
    top_level_properties: Vec<String>,
    /// `worldSaveData` child names, same reasoning.
    world_save_data_properties: Vec<String>,
    guild_count: Option<usize>,
    player_count: Option<usize>,
    pal_count: Option<usize>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct Diagnostics {
    engine_version: String,
    save_game_version: u32,
    container_format: &'static str,
    will_downgrade_to_zlib: bool,
    /// Groups whose `RawData` failed to decode. Empty is the expected state; a
    /// non-empty list is the early warning that the game format moved.
    warnings: Vec<String>,
}

#[wasm_bindgen]
pub struct SaveHandle {
    /// The primary save — normally `Level.sav`.
    container: Container,
    /// Player saves attached alongside it, keyed by the uid read out of each file.
    /// Needed because a player's container ids live in their own save, not the level
    /// (see `palsave::inventory`). Each is tens of KB against the level's megabytes,
    /// so holding a few is not a memory concern — but they are stored once, never
    /// cloned per query.
    players: BTreeMap<String, Container>,
}

#[wasm_bindgen]
pub fn open(bytes: &[u8]) -> Result<SaveHandle, JsValue> {
    let container = container::decode(bytes).map_err(|e| js_error("container_decode_failed", e))?;
    // Parse once up front so a structurally broken save fails at open() rather than
    // at the first read call.
    palsave::gvas::GvasFile::parse(&container.gvas)
        .map_err(|e| js_error("gvas_parse_failed", e))?;
    Ok(SaveHandle {
        container,
        players: BTreeMap::new(),
    })
}

#[wasm_bindgen]
impl SaveHandle {
    fn will_downgrade(&self) -> bool {
        self.container.algorithm == Algorithm::OodleMermaid
    }

    fn build_report(&self) -> Result<DiagnosticReport, JsValue> {
        let file = palsave::gvas::GvasFile::parse(&self.container.gvas)
            .map_err(|e| js_error("gvas_parse_failed", e))?;
        let h = &file.header;

        let world_save_data_properties = file
            .properties
            .iter()
            .position(|p| p.name == "worldSaveData")
            .and_then(|idx| file.materialize(idx).ok())
            .and_then(|v| {
                v.as_properties()
                    .map(|props| props.iter().map(|p| p.name.clone()).collect())
            })
            .unwrap_or_default();

        let mut warnings = Vec::new();
        let guild_count = match guilds::list(&self.container.gvas) {
            Ok(g) => Some(g.len()),
            Err(e) => {
                warnings.push(e.code().to_string());
                None
            }
        };
        let player_count = match characters::list_players(&self.container.gvas) {
            Ok(p) => Some(p.len()),
            Err(e) => {
                warnings.push(e.code().to_string());
                None
            }
        };
        let pal_count = characters::list_all_pals(&self.container.gvas)
            .ok()
            .map(|p| p.len());

        Ok(DiagnosticReport {
            engine_version: format!(
                "{}.{}.{}",
                h.engine_version_major, h.engine_version_minor, h.engine_version_patch
            ),
            save_game_version: h.save_game_version,
            package_version_ue4: h.package_version_ue4,
            package_version_ue5: h.package_version_ue5,
            save_game_class: file.save_game_type.clone(),
            container_format: self.container_format(),
            was_cnk_wrapped: self.container.was_cnk_wrapped,
            will_downgrade_to_zlib: self.will_downgrade(),
            gvas_len: self.container.gvas.len(),
            top_level_properties: file.properties.iter().map(|p| p.name.clone()).collect(),
            world_save_data_properties,
            guild_count,
            player_count,
            pal_count,
            warnings,
        })
    }

    fn container_format(&self) -> &'static str {
        match self.container.algorithm {
            Algorithm::Zlib => "PlZ",
            Algorithm::OodleMermaid => "PlM",
        }
    }

    #[wasm_bindgen]
    pub fn summary(&self) -> Result<JsValue, JsValue> {
        let file = palsave::gvas::GvasFile::parse(&self.container.gvas)
            .map_err(|e| js_error("gvas_parse_failed", e))?;
        let h = &file.header;
        to_js(&Summary {
            save_game_type: file.save_game_type.clone(),
            engine_version: format!(
                "{}.{}.{}",
                h.engine_version_major, h.engine_version_minor, h.engine_version_patch
            ),
            save_game_version: h.save_game_version,
            gvas_len: self.container.gvas.len(),
            top_level_property_count: file.properties.len(),
            container: ContainerInfo {
                format: self.container_format(),
                was_cnk_wrapped: self.container.was_cnk_wrapped,
                will_downgrade_to_zlib: self.will_downgrade(),
            },
        })
    }

    #[wasm_bindgen(js_name = listGuilds)]
    pub fn list_guilds(&self) -> Result<JsValue, JsValue> {
        let summaries = guilds::list(&self.container.gvas).map_err(guild_error)?;
        let views: Vec<GuildSummaryView> = summaries.iter().map(GuildSummaryView::from).collect();
        to_js(&views)
    }

    #[wasm_bindgen]
    pub fn guild(&self, id: &str) -> Result<JsValue, JsValue> {
        let detail = guilds::detail(&self.container.gvas, id).map_err(guild_error)?;
        to_js(&GuildDetailView {
            summary: GuildSummaryView::from(&detail.summary),
            admin_player_uid: detail.admin_player_uid,
            members: detail
                .members
                .iter()
                .map(|m| GuildMemberView {
                    player_uid: m.player_uid.clone(),
                    player_name: m.player_name.clone(),
                    last_online_real_time: m.last_online_real_time.to_string(),
                    role: m.role,
                })
                .collect(),
        })
    }

    #[wasm_bindgen(js_name = setGuildName)]
    pub fn set_guild_name(&mut self, id: &str, name: &str) -> Result<(), JsValue> {
        let edited = guilds::set_name(&self.container.gvas, id, name).map_err(guild_error)?;
        // Replace rather than keep both buffers — see the module docs on the 4 GB ceiling.
        self.container.gvas = edited;
        Ok(())
    }

    #[wasm_bindgen(js_name = listPlayers)]
    pub fn list_players(&self) -> Result<JsValue, JsValue> {
        let players = characters::list_players(&self.container.gvas).map_err(character_error)?;
        let views: Vec<PlayerSummaryView> = players.iter().map(PlayerSummaryView::from).collect();
        to_js(&views)
    }

    #[wasm_bindgen]
    pub fn player(&self, uid: &str) -> Result<JsValue, JsValue> {
        let detail = characters::player(&self.container.gvas, uid).map_err(character_error)?;
        to_js(&PlayerDetailView {
            summary: PlayerSummaryView::from(&detail.summary),
            pals: detail.pals.iter().map(PalSummaryView::from).collect(),
        })
    }

    #[wasm_bindgen(js_name = palsOf)]
    pub fn pals_of(&self, uid: &str) -> Result<JsValue, JsValue> {
        let pals = characters::pals_of(&self.container.gvas, uid).map_err(character_error)?;
        let views: Vec<PalSummaryView> = pals.iter().map(PalSummaryView::from).collect();
        to_js(&views)
    }

    /// Attaches a `Players/<uid>.sav` and returns the uid read *from the file*.
    /// The caller doesn't get to say which player this is — a mislabelled file would
    /// otherwise silently attribute one player's inventory to another.
    #[wasm_bindgen(js_name = attachPlayerSave)]
    pub fn attach_player_save(&mut self, bytes: &[u8]) -> Result<String, JsValue> {
        let container =
            container::decode(bytes).map_err(|e| js_error("container_decode_failed", e))?;
        // Rejects a Level.sav (or anything without SaveData) before it's stored.
        let uid = inventory::player_uid(&container.gvas).map_err(inventory_error)?;
        self.players.insert(uid.clone(), container);
        Ok(uid)
    }

    #[wasm_bindgen(js_name = detachPlayerSave)]
    pub fn detach_player_save(&mut self, uid: &str) {
        self.players.remove(uid);
    }

    #[wasm_bindgen(js_name = attachedPlayers)]
    pub fn attached_players(&self) -> Result<JsValue, JsValue> {
        let uids: Vec<&String> = self.players.keys().collect();
        to_js(&uids)
    }

    #[wasm_bindgen(js_name = playerInventory)]
    pub fn player_inventory(&self, uid: &str) -> Result<JsValue, JsValue> {
        let player = self.players.get(uid).ok_or_else(|| {
            js_error(
                "player_save_not_attached",
                format!("no player save attached for {uid}"),
            )
        })?;

        let inv = inventory::player_inventory(&self.container.gvas, &player.gvas)
            .map_err(inventory_error)?;

        to_js(&PlayerInventoryView {
            player_uid: inv.player_uid,
            containers: inv
                .containers
                .iter()
                .map(|c| ContainerView {
                    kind: c.kind.as_str(),
                    id: c.id.clone(),
                    slot_count: c.slot_count,
                    slots: c
                        .slots
                        .iter()
                        .map(|s| SlotView {
                            slot_index: s.slot_index,
                            count: s.count,
                            static_id: s.static_id.clone(),
                            durability: s.durability,
                            remaining_bullets: s.remaining_bullets,
                            ammo_static_id: s.ammo_static_id.clone(),
                            egg_character_id: s.egg_character_id.clone(),
                        })
                        .collect(),
                    missing: c.missing,
                })
                .collect(),
        })
    }

    /// A player's Pal box and party. Needs the same two-file pairing as
    /// `playerInventory` — the container ids live in the player's own save.
    #[wasm_bindgen(js_name = playerPalStorage)]
    pub fn player_pal_storage(&self, uid: &str) -> Result<JsValue, JsValue> {
        let player = self.players.get(uid).ok_or_else(|| {
            js_error(
                "player_save_not_attached",
                format!("no player save attached for {uid}"),
            )
        })?;

        let storage = inventory::player_pal_storage(&self.container.gvas, &player.gvas)
            .map_err(inventory_error)?;

        to_js(&PlayerPalStorageView {
            player_uid: storage.player_uid,
            containers: storage
                .containers
                .iter()
                .map(|c| PalContainerView {
                    kind: c.kind.as_str(),
                    id: c.id.clone(),
                    slot_count: c.slot_count,
                    slots: c
                        .slots
                        .iter()
                        .map(|s| PalSlotView {
                            slot_index: s.slot_index,
                            instance_id: s.instance_id.clone(),
                            pal: s.pal.as_ref().map(PalSummaryView::from),
                        })
                        .collect(),
                    missing: c.missing,
                })
                .collect(),
        })
    }

    #[wasm_bindgen]
    pub fn diagnostics(&self) -> Result<JsValue, JsValue> {
        let file = palsave::gvas::GvasFile::parse(&self.container.gvas)
            .map_err(|e| js_error("gvas_parse_failed", e))?;
        let h = &file.header;

        let warnings = match guilds::list(&self.container.gvas) {
            Ok(_) => Vec::new(),
            Err(e) => vec![format!("{}: {}", e.code(), e)],
        };

        to_js(&Diagnostics {
            engine_version: format!(
                "{}.{}.{}",
                h.engine_version_major, h.engine_version_minor, h.engine_version_patch
            ),
            save_game_version: h.save_game_version,
            container_format: self.container_format(),
            will_downgrade_to_zlib: self.will_downgrade(),
            warnings,
        })
    }

    /// A shareable diagnostic report — format structure and counts only, no personal
    /// data. This is how a user reports breakage without sending their world.
    #[wasm_bindgen(js_name = diagnosticReport)]
    pub fn diagnostic_report(&self) -> Result<JsValue, JsValue> {
        to_js(&self.build_report()?)
    }

    #[wasm_bindgen(js_name = setPalStat)]
    pub fn set_pal_stat(
        &mut self,
        instance_id: &str,
        stat: &str,
        value: f64,
    ) -> Result<(), JsValue> {
        let stat = parse_pal_stat(stat)?;
        let edited =
            characters::set_pal_stat(&self.container.gvas, instance_id, stat, value as i64)
                .map_err(character_error)?;
        self.container.gvas = edited;
        Ok(())
    }

    #[wasm_bindgen(js_name = setPalNickname)]
    pub fn set_pal_nickname(&mut self, instance_id: &str, nickname: &str) -> Result<(), JsValue> {
        let edited = characters::set_pal_nickname(&self.container.gvas, instance_id, nickname)
            .map_err(character_error)?;
        self.container.gvas = edited;
        Ok(())
    }

    #[wasm_bindgen(js_name = setPlayerStat)]
    pub fn set_player_stat(&mut self, uid: &str, stat: &str, value: f64) -> Result<(), JsValue> {
        let stat = parse_player_stat(stat)?;
        let edited = characters::set_player_stat(&self.container.gvas, uid, stat, value as i64)
            .map_err(character_error)?;
        self.container.gvas = edited;
        Ok(())
    }

    /// The one call that returns something proportional to save size. Re-compresses
    /// the current buffer into a `.sav` container.
    #[wasm_bindgen]
    pub fn export(&self) -> Result<Vec<u8>, JsValue> {
        // Structural check before handing bytes to a caller who may write them over a
        // real save. Refuse to return a buffer that doesn't re-parse.
        palsave::edit::verify_reparses(&self.container.gvas)
            .map_err(|e| js_error("export_verification_failed", e))?;
        Ok(container::encode(&self.container.gvas, &self.container))
    }
}
