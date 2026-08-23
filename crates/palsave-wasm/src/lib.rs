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
use serde::Serialize;
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
    container: Container,
}

#[wasm_bindgen]
pub fn open(bytes: &[u8]) -> Result<SaveHandle, JsValue> {
    let container = container::decode(bytes).map_err(|e| js_error("container_decode_failed", e))?;
    // Parse once up front so a structurally broken save fails at open() rather than
    // at the first read call.
    palsave::gvas::GvasFile::parse(&container.gvas)
        .map_err(|e| js_error("gvas_parse_failed", e))?;
    Ok(SaveHandle { container })
}

#[wasm_bindgen]
impl SaveHandle {
    fn will_downgrade(&self) -> bool {
        self.container.algorithm == Algorithm::OodleMermaid
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
