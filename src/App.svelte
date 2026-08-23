<script lang="ts">
  import Dropzone from './lib/components/Dropzone.svelte';
  import Inspector from './lib/components/Inspector.svelte';
  import Guilds from './lib/components/Guilds.svelte';
  import Players from './lib/components/Players.svelte';
  import Inventory from './lib/components/Inventory.svelte';
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
  let attachedPlayers = $state<string[]>([]);
  let tab = $state<'inspector' | 'players' | 'inventory' | 'guilds'>('inspector');
  let busy = $state(false);
  let error = $state<SaveError | null>(null);
  let edited = $state(false);
  let exporting = $state(false);

  const PLAYER_SAVE_CLASS = '/Script/Pal.PalWorldPlayerSaveGame';

  /**
   * Routes dropped files by what they actually are, not what they're called: each is
   * probed with `open()` and identified by its save class. A file named Level.sav
   * that is really a player save would otherwise be opened as the primary and every
   * screen would come up empty.
   *
   * The primary is the first non-player save; player saves are attached alongside.
   * Opening a player save on its own still works — it just becomes the primary and
   * the level-only screens stay empty, which is the pre-existing behaviour.
   */
  async function openFiles(files: File[]) {
    busy = true;
    error = null;
    try {
      const probed = await Promise.all(
        files.map(async (file) => ({ file, bytes: await file.arrayBuffer() })),
      );

      // Identify each without committing to one: open() replaces the handle, so the
      // primary has to be chosen before anything is attached.
      const classified: { file: File; bytes: ArrayBuffer; isPlayer: boolean }[] = [];
      for (const { file, bytes } of probed) {
        // A copy per probe: open() transfers, and we may need these bytes again.
        const probe = await client.open(bytes.slice(0));
        classified.push({ file, bytes, isPlayer: probe.save_game_type === PLAYER_SAVE_CLASS });
      }

      const primary = classified.find((c) => !c.isPlayer) ?? classified[0];
      summary = await client.open(primary.bytes);
      fileName = primary.file.name;
      edited = false;

      attachedPlayers = [];
      for (const candidate of classified) {
        if (candidate === primary || !candidate.isPlayer) continue;
        try {
          const uid = await client.attachPlayerSave(candidate.bytes);
          attachedPlayers = [...attachedPlayers, uid];
        } catch {
          // One unreadable player save shouldn't stop the level from opening.
        }
      }

      diagnostics = await client.diagnostics();
      guilds = await loadGuilds();
      players = await loadPlayers();
      tab = 'inspector';
    } catch (e) {
      error = e as SaveError;
      summary = null;
      fileName = null;
      attachedPlayers = [];
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
    attachedPlayers = [];
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
      <code data-testid="error-code">{error.code}</code>
      {error.message}
    </p>
  {/if}

  {#if !summary}
    <Dropzone onfiles={openFiles} {busy} />
  {:else}
    <div class="filebar" data-testid="filebar">
      <div>
        <strong data-testid="filename">{fileName}</strong>
        {#if edited}<span class="badge" data-testid="dirty">unsaved changes</span>{/if}
      </div>
      <div class="actions">
        <button onclick={download} disabled={exporting} class="primary" data-testid="download">
          {exporting ? 'Preparing…' : 'Download .sav'}
        </button>
        <button onclick={reset} data-testid="close">Close</button>
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
      <button class:active={tab === 'inspector'} onclick={() => (tab = 'inspector')} data-testid="tab-inspector">Inspector</button>
      <button
        class:active={tab === 'players'}
        onclick={() => (tab = 'players')}
        data-testid="tab-players"
        disabled={players.length === 0}
      >
        Players {players.length > 0 ? `(${players.length})` : ''}
      </button>
      <button class:active={tab === 'inventory'} onclick={() => (tab = 'inventory')} data-testid="tab-inventory">
        Inventory {attachedPlayers.length > 0 ? `(${attachedPlayers.length})` : ''}
      </button>
      <button class:active={tab === 'guilds'} onclick={() => (tab = 'guilds')} disabled={guilds.length === 0} data-testid="tab-guilds">
        Guilds {guilds.length > 0 ? `(${guilds.length})` : ''}
      </button>
    </nav>

    {#if tab === 'inspector'}
      <Inspector {summary} {diagnostics} />
    {:else if tab === 'players'}
      <Players {client} {players} />
    {:else if tab === 'inventory'}
      <Inventory {client} {attachedPlayers} />
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
