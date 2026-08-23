<script lang="ts">
  import Dropzone from './lib/components/Dropzone.svelte';
  import Inspector from './lib/components/Inspector.svelte';
  import Guilds from './lib/components/Guilds.svelte';
  import Players from './lib/components/Players.svelte';
  import { SaveClient } from './lib/worker/client';
  import type {
    Diagnostics,
    GuildSummary,
    PlayerSummary,
    SaveError,
    SaveSummary,
  } from './lib/save-types';

  // One worker for the app's lifetime. The save itself lives in wasm memory inside
  // it — deliberately not in any store here (CLAUDE.md's boundary rule).
  const client = new SaveClient();

  let fileName = $state<string | null>(null);
  let summary = $state<SaveSummary | null>(null);
  let diagnostics = $state<Diagnostics | null>(null);
  let guilds = $state<GuildSummary[]>([]);
  let players = $state<PlayerSummary[]>([]);
  let tab = $state<'inspector' | 'players' | 'guilds'>('inspector');
  let busy = $state(false);
  let error = $state<SaveError | null>(null);
  let edited = $state(false);
  let exporting = $state(false);

  async function openFile(file: File) {
    busy = true;
    error = null;
    try {
      const bytes = await file.arrayBuffer();
      summary = await client.open(bytes); // transfers `bytes`; it's detached after this
      fileName = file.name;
      edited = false;
      diagnostics = await client.diagnostics();
      guilds = await loadGuilds();
      players = await loadPlayers();
      tab = 'inspector';
    } catch (e) {
      error = e as SaveError;
      summary = null;
      fileName = null;
    } finally {
      busy = false;
    }
  }

  /** Only a Level.sav has these maps. A player save or LevelMeta legitimately has
   *  neither, so a structural miss yields an empty list rather than an error banner. */
  const MISSING_MAP_CODES = ['no_group_map', 'not_a_level_save', 'map_not_found'];

  function isMissingMap(e: unknown): boolean {
    return MISSING_MAP_CODES.includes((e as SaveError)?.code);
  }

  async function loadGuilds(): Promise<GuildSummary[]> {
    try {
      return await client.listGuilds();
    } catch (e) {
      if (isMissingMap(e)) return [];
      throw e;
    }
  }

  async function loadPlayers(): Promise<PlayerSummary[]> {
    try {
      return await client.listPlayers();
    } catch (e) {
      if (isMissingMap(e)) return [];
      throw e;
    }
  }

  async function refreshAfterEdit() {
    edited = true;
    guilds = await loadGuilds();
    summary = await client.summary();
  }

  async function download() {
    if (!summary || !fileName) return;
    exporting = true;
    error = null;
    try {
      const buffer = await client.export();
      const url = URL.createObjectURL(new Blob([buffer], { type: 'application/octet-stream' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = fileName;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      error = e as SaveError;
    } finally {
      exporting = false;
    }
  }

  function reset() {
    client.close();
    summary = null;
    diagnostics = null;
    guilds = [];
    players = [];
    tab = 'inspector';
    fileName = null;
    edited = false;
    error = null;
  }
</script>

<main>
  <header>
    <h1>Palworld save editor</h1>
    <p class="tagline">Runs entirely in your browser. No uploads, no server, no telemetry.</p>
  </header>

  {#if error}
    <p class="banner danger">
      <code>{error.code}</code>
      {error.message}
    </p>
  {/if}

  {#if !summary}
    <Dropzone onfile={openFile} {busy} />
  {:else}
    <div class="filebar">
      <div>
        <strong>{fileName}</strong>
        {#if edited}<span class="badge">unsaved changes</span>{/if}
      </div>
      <div class="actions">
        <button onclick={download} disabled={exporting} class="primary">
          {exporting ? 'Preparing…' : 'Download .sav'}
        </button>
        <button onclick={reset}>Close</button>
      </div>
    </div>

    <!-- Non-dismissable: writing to a save the game has open loses the edit at best. -->
    <p class="banner warn">
      Close Palworld — or stop your server — before replacing a save file.
    </p>

    {#if summary.container.will_downgrade_to_zlib}
      <p class="banner warn">
        This save uses Oodle compression (PlM). No open-source Oodle compressor exists,
        so it will be written back as zlib (PlZ). The file will be larger and won't be
        byte-identical. The game reads both formats and re-encodes on its next autosave.
      </p>
    {/if}

    <nav class="tabs">
      <button class:active={tab === 'inspector'} onclick={() => (tab = 'inspector')}>Inspector</button>
      <button
        class:active={tab === 'players'}
        onclick={() => (tab = 'players')}
        disabled={players.length === 0}
      >
        Players {players.length > 0 ? `(${players.length})` : ''}
      </button>
      <button class:active={tab === 'guilds'} onclick={() => (tab = 'guilds')} disabled={guilds.length === 0}>
        Guilds {guilds.length > 0 ? `(${guilds.length})` : ''}
      </button>
    </nav>

    {#if tab === 'inspector'}
      <Inspector {summary} {diagnostics} />
    {:else if tab === 'players'}
      <Players {client} {players} />
    {:else}
      <Guilds {client} {guilds} onedited={refreshAfterEdit} />
    {/if}
  {/if}

  <footer>
    <p>
      Exports download as a new file. Writing back in place — with an automatic backup
      first — isn't implemented yet; replace the original yourself, and keep a copy.
    </p>
  </footer>
</main>

<style>
  main {
    max-width: 62rem;
    margin: 0 auto;
    padding: 2rem 1.25rem 4rem;
  }
  header h1 {
    margin: 0;
    font-size: 1.5rem;
  }
  .tagline {
    margin: 0.3rem 0 1.75rem;
    color: var(--muted);
    font-size: 0.9rem;
  }
  .filebar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 1rem;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
  .badge {
    font-size: 0.75rem;
    background: var(--warn-soft);
    color: var(--warn);
    padding: 0.15em 0.5em;
    border-radius: 4px;
    margin-left: 0.5rem;
  }
  .banner {
    padding: 0.6rem 0.9rem;
    border-radius: 8px;
    font-size: 0.875rem;
    margin: 0 0 0.75rem;
    border: 1px solid transparent;
  }
  .banner.warn {
    background: var(--warn-soft);
    color: var(--warn);
    border-color: var(--warn-border);
  }
  .banner.danger {
    background: var(--danger-soft);
    color: var(--danger);
    border-color: var(--danger-border);
  }
  .tabs {
    display: flex;
    gap: 0.25rem;
    border-bottom: 1px solid var(--border);
    margin: 1.5rem 0 1.25rem;
  }
  .tabs button {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    padding: 0.5rem 0.85rem;
    cursor: pointer;
    color: var(--muted);
    font: inherit;
    margin-bottom: -1px;
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tabs button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  footer {
    margin-top: 3rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 0.8rem;
  }
  code {
    background: var(--code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
  }
</style>
