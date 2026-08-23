<script lang="ts">
  /**
   * Screen 5: preview moving a player from another world into this one.
   *
   * Read-only, and deliberately so. Nothing here writes: it answers "what would move,
   * and what would break" and stops. Applying a migration is a separate change with its
   * own gates, because a migration touches four maps and an array at once and the
   * failure that matters is silent — two characters ending up with one identity.
   *
   * The open save is always the *destination*. You are editing it; you attach the world
   * to import from. Stating that once here means no call site has to remember which
   * argument is which.
   */
  import type { ConflictView, MigrationPlan, SaveError } from '../save-types';
  import type { SaveClient } from '../worker/client';

  let { client }: { client: SaveClient } = $props();

  let sourceLoaded = $state(false);
  let sourcePlayers = $state<string[]>([]);
  let attachedSourcePlayers = $state<string[]>([]);
  let selectedUid = $state<string | null>(null);
  let plan = $state<MigrationPlan | null>(null);
  let error = $state<SaveError | null>(null);
  let busy = $state(false);

  const CONFLICT_LABELS: Record<ConflictView['code'], string> = {
    player_exists: 'A player with this uid is already in this save',
    pal_instance_exists: 'A Pal with this instance id is already here',
    container_exists: 'A container with this id is already here',
    dynamic_item_exists: 'An item state row with this id is already here',
    guild_missing: 'Their guild does not exist here',
  };

  /** `guild_missing` is a dangling reference, not a duplicate — shown, not counted. */
  function isBlocking(c: ConflictView) {
    return c.code !== 'guild_missing';
  }

  async function onSourceWorld(event: Event) {
    const file = (event.target as HTMLInputElement).files?.[0];
    if (!file) return;
    busy = true;
    error = null;
    try {
      await client.attachSourceWorld(await file.arrayBuffer());
      sourceLoaded = true;
      sourcePlayers = await client.sourcePlayers();
    } catch (e) {
      error = e as SaveError;
      sourceLoaded = false;
    } finally {
      busy = false;
    }
  }

  async function onSourcePlayer(event: Event) {
    const files = Array.from((event.target as HTMLInputElement).files ?? []);
    if (files.length === 0) return;
    busy = true;
    error = null;
    try {
      for (const file of files) {
        const uid = await client.attachSourcePlayer(await file.arrayBuffer());
        if (!attachedSourcePlayers.includes(uid)) {
          attachedSourcePlayers = [...attachedSourcePlayers, uid];
        }
      }
    } catch (e) {
      error = e as SaveError;
    } finally {
      busy = false;
    }
  }

  async function preview(uid: string) {
    selectedUid = uid;
    plan = null;
    error = null;
    busy = true;
    try {
      plan = await client.migrationPlan(uid);
    } catch (e) {
      error = e as SaveError;
    } finally {
      busy = false;
    }
  }

  async function reset() {
    await client.clearSource();
    sourceLoaded = false;
    sourcePlayers = [];
    attachedSourcePlayers = [];
    selectedUid = null;
    plan = null;
    error = null;
  }
</script>

<section data-testid="migrate">
  <p class="lede">
    Preview moving a player — with their Pals, inventories and Pal box — out of another
    world and into the save that's currently open. <strong>Nothing is written yet</strong>:
    this shows what would move and what it would collide with.
  </p>

  <ol class="steps">
    <li>
      <h4>1. The world to migrate from</h4>
      <label class="filebtn">
        <input
          type="file"
          accept=".sav"
          onchange={onSourceWorld}
          data-testid="source-world-input"
        />
        Choose that world's <code>Level.sav</code>
      </label>
      {#if sourceLoaded}
        <p class="ok" data-testid="source-loaded">
          Loaded — {sourcePlayers.length}
          {sourcePlayers.length === 1 ? 'player' : 'players'} in that world.
        </p>
      {/if}
    </li>

    {#if sourceLoaded}
      <li>
        <h4>2. That player's own save file</h4>
        <p class="muted small">
          Their container ids live in their own file, not the level — the same reason the
          Pals &amp; items tab needs it.
        </p>
        <label class="filebtn">
          <input
            type="file"
            accept=".sav"
            multiple
            onchange={onSourcePlayer}
            data-testid="source-player-input"
          />
          Choose <code>Players/&lt;uid&gt;.sav</code>
        </label>
      </li>
    {/if}

    {#if attachedSourcePlayers.length > 0}
      <li>
        <h4>3. Who to move</h4>
        <ul class="uids">
          {#each attachedSourcePlayers as uid (uid)}
            <li>
              <button
                class:selected={uid === selectedUid}
                onclick={() => preview(uid)}
                data-testid="preview-{uid}"
              >
                <span class="mono">{uid}</span>
                {#if !sourcePlayers.includes(uid)}
                  <span class="warn small">— not a player of that world</span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      </li>
    {/if}
  </ol>

  {#if error}
    <p class="error"><code>{error.code}</code> {error.message}</p>
  {/if}
  {#if busy}
    <p class="muted">Working…</p>
  {/if}

  {#if plan}
    <div class="plan" data-testid="migration-plan">
      <h4>Would move {plan.row_count} rows</h4>
      <dl>
        <div><dt>Pals</dt><dd>{plan.pal_count}</dd></div>
        <div><dt>Item containers</dt><dd>{plan.item_container_count}</dd></div>
        <div><dt>Pal containers</dt><dd>{plan.pal_container_count}</dd></div>
        <div><dt>Item state rows</dt><dd>{plan.dynamic_item_count}</dd></div>
      </dl>

      {#if plan.conflicts.length === 0}
        <p class="ok">No collisions — every identity is free in this save.</p>
      {:else}
        <h4>
          {plan.blocking_count} blocking
          {plan.blocking_count === 1 ? 'collision' : 'collisions'}
        </h4>
        <ul class="conflicts" data-testid="conflicts">
          {#each plan.conflicts as conflict (conflict.code + conflict.id)}
            <li class:blocking={isBlocking(conflict)}>
              <span class="what">{CONFLICT_LABELS[conflict.code] ?? conflict.code}</span>
              <code>{conflict.id}</code>
            </li>
          {/each}
        </ul>
        <p class="muted small">
          A blocking collision means both worlds already contain something with that
          identity. Copying regardless would leave this save with two of them, and
          nothing downstream would notice — so applying a migration will have to renumber
          them rather than overwrite.
        </p>
      {/if}

      <p class="muted small">
        Applying isn't implemented yet. This preview is the part that has to be right
        first.
      </p>
    </div>
  {/if}

  {#if sourceLoaded}
    <button class="reset" onclick={reset} data-testid="clear-source">
      Forget that world
    </button>
  {/if}
</section>

<style>
  .lede {
    margin: 0 0 1.25rem;
    max-width: 46rem;
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1.25rem;
  }
  .steps h4 {
    margin: 0 0 0.4rem;
    font-size: 0.95rem;
  }
  .filebtn {
    display: inline-block;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.9rem;
    cursor: pointer;
    font-size: 0.9rem;
  }
  .filebtn input {
    display: none;
  }
  .uids {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
    max-width: 34rem;
  }
  .uids button {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.6rem 0.9rem;
    background: none;
    border: none;
    border-bottom: 1px solid var(--border);
    cursor: pointer;
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
  }
  .uids li:last-child button {
    border-bottom: none;
  }
  .uids button.selected {
    background: var(--surface-hover);
  }
  .plan {
    margin-top: 1.5rem;
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1rem 1.25rem;
  }
  .plan h4 {
    margin: 0 0 0.6rem;
    font-size: 1rem;
  }
  dl {
    margin: 0 0 1rem;
    display: grid;
    gap: 0.3rem;
  }
  dl div {
    display: grid;
    grid-template-columns: minmax(9rem, 14rem) 1fr;
    gap: 1rem;
  }
  dt {
    color: var(--muted);
    font-size: 0.9rem;
  }
  dd {
    margin: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.9rem;
  }
  .conflicts {
    list-style: none;
    margin: 0 0 0.75rem;
    padding: 0;
    display: grid;
    gap: 0.35rem;
  }
  .conflicts li {
    font-size: 0.85rem;
    color: var(--muted);
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    align-items: baseline;
  }
  .conflicts li.blocking .what {
    color: var(--warn);
  }
  .reset {
    margin-top: 1.5rem;
    background: none;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.45rem 0.9rem;
    cursor: pointer;
    color: var(--muted);
    font: inherit;
    font-size: 0.85rem;
  }
  .ok {
    color: var(--ok);
    font-size: 0.9rem;
    margin: 0.5rem 0 0;
  }
  .warn {
    color: var(--warn);
  }
  .error {
    color: var(--danger);
    font-size: 0.9rem;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85rem;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.85em;
  }
  code {
    background: var(--code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
    font-size: 0.85em;
    overflow-wrap: anywhere;
  }
</style>
