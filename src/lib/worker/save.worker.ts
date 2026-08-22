/// <reference lib="webworker" />
/**
 * Hosts the wasm module and owns the `SaveHandle`. The main thread never touches
 * either — it sends commands and receives view models.
 *
 * Keeping this off the main thread matters for more than tidiness: opening
 * `Level.sav` decompresses ~8.5 MB through Oodle and walks the property tree, which
 * would visibly jank the UI if it ran between frames.
 */

import init, { open as wasmOpen, type SaveHandle } from '../wasm/palsave_wasm.js';
import wasmUrl from '../wasm/palsave_wasm_bg.wasm?url';
import type { Request, Response } from './protocol';
import type { SaveError } from '../save-types';

let ready: Promise<unknown> | null = null;
let handle: SaveHandle | null = null;

function ensureReady() {
  // One init for the worker's lifetime; concurrent opens await the same promise.
  ready ??= init({ module_or_path: wasmUrl });
  return ready;
}

/** Anything thrown across the wasm boundary already has `{ code, message }`. A throw
 * from our own glue might not, so normalise it rather than leaking `undefined` codes
 * into the UI's exhaustive `switch`. */
function toSaveError(e: unknown): SaveError {
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return e as SaveError;
  }
  return { code: 'gvas_parse_failed', message: e instanceof Error ? e.message : String(e) };
}

function requireHandle(): SaveHandle {
  if (!handle) {
    throw { code: 'gvas_parse_failed', message: 'no save is open' } satisfies SaveError;
  }
  return handle;
}

function closeHandle() {
  // Explicitly free wasm-side memory rather than waiting for GC to notice — the
  // buffer is megabytes and a replaced-but-unfreed handle is a real leak.
  handle?.free();
  handle = null;
}

async function handleRequest(req: Request): Promise<{ value: unknown; transfer: Transferable[] }> {
  await ensureReady();

  switch (req.kind) {
    case 'open': {
      closeHandle();
      handle = wasmOpen(new Uint8Array(req.bytes));
      return { value: handle.summary(), transfer: [] };
    }
    case 'summary':
      return { value: requireHandle().summary(), transfer: [] };
    case 'listGuilds':
      return { value: requireHandle().listGuilds(), transfer: [] };
    case 'guild':
      return { value: requireHandle().guild(req.guildId), transfer: [] };
    case 'setGuildName':
      requireHandle().setGuildName(req.guildId, req.name);
      return { value: null, transfer: [] };
    case 'diagnostics':
      return { value: requireHandle().diagnostics(), transfer: [] };
    case 'export': {
      const bytes = requireHandle().export();
      // `bytes` is a fresh copy owned by JS; hand its buffer over rather than cloning.
      const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      return { value: buffer, transfer: [buffer] };
    }
    case 'close':
      closeHandle();
      return { value: null, transfer: [] };
  }
}

self.onmessage = async (event: MessageEvent<Request>) => {
  const req = event.data;
  try {
    const { value, transfer } = await handleRequest(req);
    const response: Response = { id: req.id, ok: true, value };
    self.postMessage(response, transfer);
  } catch (e) {
    const response: Response = { id: req.id, ok: false, error: toSaveError(e) };
    self.postMessage(response);
  }
};
