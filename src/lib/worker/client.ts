/**
 * Main-thread client for the save worker: a promise-per-request wrapper over
 * `postMessage`.
 *
 * Deliberately hand-rolled rather than pulling in Comlink — the protocol is eight
 * commands and the one thing that actually needs care (transferring ArrayBuffers
 * instead of cloning them) is more explicit written out than hidden behind a proxy.
 */

import type { Request, ResultOf, Response } from './protocol';
import type { SaveError } from '../save-types';

type Pending = { resolve: (v: unknown) => void; reject: (e: SaveError) => void };

export class SaveClient {
  #worker: Worker;
  #pending = new Map<number, Pending>();
  #nextId = 1;

  constructor() {
    this.#worker = new Worker(new URL('./save.worker.ts', import.meta.url), { type: 'module' });
    this.#worker.onmessage = (event: MessageEvent<Response>) => {
      const res = event.data;
      const pending = this.#pending.get(res.id);
      if (!pending) return;
      this.#pending.delete(res.id);
      if (res.ok) pending.resolve(res.value);
      else pending.reject(res.error);
    };
    this.#worker.onerror = (event) => {
      // The worker died (module load failure, panic). Every in-flight request is
      // unrecoverable; fail them all rather than leaving the UI spinning forever.
      const error: SaveError = {
        code: 'gvas_parse_failed',
        message: event.message || 'save worker failed',
      };
      for (const [, pending] of this.#pending) pending.reject(error);
      this.#pending.clear();
    };
  }

  #send<K extends Request['kind']>(
    req: Extract<Request, { kind: K }>,
    transfer: Transferable[] = [],
  ): Promise<ResultOf[K]> {
    return new Promise((resolve, reject) => {
      this.#pending.set(req.id, { resolve: resolve as (v: unknown) => void, reject });
      this.#worker.postMessage(req, transfer);
    });
  }

  #id() {
    return this.#nextId++;
  }

  /** Takes ownership of `bytes` — the caller's ArrayBuffer is detached. */
  open(bytes: ArrayBuffer) {
    return this.#send({ id: this.#id(), kind: 'open', bytes }, [bytes]);
  }

  summary() {
    return this.#send({ id: this.#id(), kind: 'summary' });
  }

  listGuilds() {
    return this.#send({ id: this.#id(), kind: 'listGuilds' });
  }

  guild(guildId: string) {
    return this.#send({ id: this.#id(), kind: 'guild', guildId });
  }

  setGuildName(guildId: string, name: string) {
    return this.#send({ id: this.#id(), kind: 'setGuildName', guildId, name });
  }

  diagnostics() {
    return this.#send({ id: this.#id(), kind: 'diagnostics' });
  }

  export() {
    return this.#send({ id: this.#id(), kind: 'export' });
  }

  close() {
    return this.#send({ id: this.#id(), kind: 'close' });
  }

  terminate() {
    this.#worker.terminate();
    this.#pending.clear();
  }
}
