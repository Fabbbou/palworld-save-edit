/**
 * Typed shapes for the view models `palsave-wasm` returns.
 *
 * wasm-bindgen declares these methods as `any` because they cross the boundary as
 * `JsValue` (serde-wasm-bindgen). These interfaces are the hand-maintained
 * counterpart to the `#[derive(Serialize)]` structs in `crates/palsave-wasm/src/lib.rs`
 * — if you change a field there, change it here. Nothing enforces that automatically.
 *
 * Note what is deliberately absent: there is no type for "the save". The save never
 * crosses into JS (see CLAUDE.md's boundary rule); JS holds an opaque `SaveHandle`
 * and receives only these small models.
 */

export interface ContainerInfo {
  /** Container format found on open: `PlZ` (zlib) or `PlM` (Oodle Mermaid). */
  format: 'PlZ' | 'PlM';
  was_cnk_wrapped: boolean;
  /**
   * True when `export()` will write a different format than the save was opened
   * from. No open-source Oodle *compressor* exists, so a PlM save is written back as
   * PlZ — larger, and not byte-identical. The game reads both and re-encodes on its
   * next autosave. The UI must surface this before any in-place write.
   */
  will_downgrade_to_zlib: boolean;
}

export interface SaveSummary {
  save_game_type: string;
  /** `major.minor.patch`, e.g. `"5.1.1"`. */
  engine_version: string;
  save_game_version: number;
  /** Decompressed GVAS size in bytes — a number, not the bytes. */
  gvas_len: number;
  top_level_property_count: number;
  container: ContainerInfo;
}

export interface GuildSummary {
  /** 32 lowercase hex chars. Opaque handle — pass back to `guild()` / `setGuildName()`. */
  id: string;
  /** e.g. `"EPalGroupType::Guild"`, `"EPalGroupType::Organization"`. */
  group_type: string;
  name: string;
  member_count: number;
  base_camp_level: number;
  /** Number of Pals owned by this guild. */
  pal_count: number;
}

export interface GuildMember {
  player_uid: string;
  player_name: string;
  /**
   * Unreal `FDateTime` ticks, as a decimal string. It exceeds 2^53 in practice, so
   * it crosses as a string rather than a JS `number` — parsing it into one loses
   * precision silently. Use `BigInt(...)` if you need arithmetic.
   */
  last_online_real_time: string;
  /** Present only on the newer guild tail shape; `null` on older saves. */
  role: number | null;
}

export interface GuildDetail {
  summary: GuildSummary;
  admin_player_uid: string | null;
  members: GuildMember[];
}

/**
 * A player character from `CharacterSaveParameterMap`.
 *
 * Nearly every field is nullable, and that is the data's shape rather than
 * defensiveness: Palworld only writes a `SaveParameter` field once it differs from
 * the default, and renames fields between versions. Render a missing value as "—",
 * never as 0.
 *
 * `exp`, `hp` and `shield_hp` are decimal **strings**: they are i64 game stats
 * (fixed-point HP runs to seven figures) and would lose precision as JS numbers.
 * Use `BigInt` for arithmetic.
 */
export interface PlayerSummary {
  /** 32 hex chars in Unreal's display convention — this is also the name of the
   *  player's own `Players/<uid>.sav` file. */
  uid: string;
  instance_id: string;
  nickname: string | null;
  level: number | null;
  exp: string | null;
  hp: string | null;
  shield_hp: string | null;
  full_stomach: number | null;
  /** Pals naming this player as owner. Base-camp Pals have no owner and aren't
   *  counted here. */
  pal_count: number;
}

export interface PalSummary {
  instance_id: string;
  owner_player_uid: string | null;
  /** Species id, e.g. `ChickenPal`. */
  character_id: string | null;
  nickname: string | null;
  gender: string | null;
  level: number | null;
  exp: string | null;
  hp: string | null;
  /** The three IVs, 0–100. */
  talent_hp: number | null;
  talent_shot: number | null;
  talent_defense: number | null;
  passive_skills: string[];
  friendship_point: number | null;
  /** Condensation rank; absent on most Pals. */
  rank: number | null;
  sanity_value: number | null;
  /** The underlying field only exists on rare Pals, so `false` means "not rare",
   *  not "unknown". */
  is_rare: boolean;
}

export interface PlayerDetail {
  summary: PlayerSummary;
  pals: PalSummary[];
}

export interface Diagnostics {
  engine_version: string;
  save_game_version: number;
  container_format: 'PlZ' | 'PlM';
  will_downgrade_to_zlib: boolean;
  /**
   * Empty is the expected state. A non-empty list is the early warning that the game
   * format moved and some decoder fell back to opaque bytes.
   */
  warnings: string[];
}

/**
 * Every error thrown across the boundary. Branch on `code` — it's stable. `message`
 * is for display and bug reports only.
 */
export interface SaveError {
  code:
    | 'container_decode_failed'
    | 'gvas_parse_failed'
    | 'rawdata_decode_failed'
    | 'edit_failed'
    | 'no_group_map'
    | 'not_a_level_save'
    | 'map_not_found'
    | 'player_not_found'
    | 'malformed_uid'
    | 'guild_not_found'
    | 'not_a_named_guild'
    | 'malformed_guild_id'
    | 'export_verification_failed'
    | 'serialization_failed';
  message: string;
}

export function isSaveError(e: unknown): e is SaveError {
  return typeof e === 'object' && e !== null && 'code' in e && 'message' in e;
}
