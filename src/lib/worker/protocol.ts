/**
 * Message protocol between the main thread and the save worker.
 *
 * The worker owns the `SaveHandle`, which owns the decompressed save in wasm linear
 * memory. Nothing resembling the save itself is ever in a message: requests are
 * commands, responses are the small view models from `save-types.ts`. The two
 * exceptions are the raw file bytes on `open` and the exported bytes on `export`,
 * and both are **transferred** (zero-copy, sender loses the buffer) rather than
 * structured-cloned, so a multi-megabyte `Level.sav` is never duplicated.
 */

import type {
  Diagnostics,
  GuildDetail,
  GuildSummary,
  PalSummary,
  PlayerDetail,
  PlayerInventory,
  PlayerSummary,
  SaveError,
  SaveSummary,
} from '../save-types';

export type Request =
  | { id: number; kind: 'open'; bytes: ArrayBuffer }
  | { id: number; kind: 'summary' }
  | { id: number; kind: 'listGuilds' }
  | { id: number; kind: 'guild'; guildId: string }
  | { id: number; kind: 'setGuildName'; guildId: string; name: string }
  | { id: number; kind: 'listPlayers' }
  | { id: number; kind: 'player'; uid: string }
  | { id: number; kind: 'palsOf'; uid: string }
  | { id: number; kind: 'attachPlayerSave'; bytes: ArrayBuffer }
  | { id: number; kind: 'detachPlayerSave'; uid: string }
  | { id: number; kind: 'attachedPlayers' }
  | { id: number; kind: 'playerInventory'; uid: string }
  | { id: number; kind: 'diagnostics' }
  | { id: number; kind: 'export' }
  | { id: number; kind: 'close' };

/** Maps each request kind to what its `ok` response carries. */
export interface ResultOf {
  open: SaveSummary;
  summary: SaveSummary;
  listGuilds: GuildSummary[];
  guild: GuildDetail;
  setGuildName: null;
  listPlayers: PlayerSummary[];
  player: PlayerDetail;
  palsOf: PalSummary[];
  attachPlayerSave: string;
  detachPlayerSave: null;
  attachedPlayers: string[];
  playerInventory: PlayerInventory;
  diagnostics: Diagnostics;
  export: ArrayBuffer;
  close: null;
}

export type Response =
  | { id: number; ok: true; value: unknown }
  | { id: number; ok: false; error: SaveError };
