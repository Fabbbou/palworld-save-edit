<script lang="ts">
  /**
   * Screen 4: a player's inventories. Read-only.
   *
   * Needs two files — the ids come from the player's own save, the contents from
   * Level.sav — so this screen is empty until a `Players/*.sav` is attached. That
   * empty state has to explain itself, otherwise the tab just looks broken.
   */
  import type { ContainerView, PlayerInventory, SaveError } from '../save-types';
  import type { SaveClient } from '../worker/client';

  let {
    client,
    attachedPlayers,
  }: {
    client: SaveClient;
    /** Uids of player saves currently attached to the handle. */
    attachedPlayers: string[];
  } = $props();

  let selectedUid = $state<string | null>(null);
  let inventory = $state<PlayerInventory | null>(null);
  let error = $state<SaveError | null>(null);
  let loading = $state(false);

  const KIND_LABELS: Record<ContainerView['kind'], string> = {
    common: 'Inventory',
    essential: 'Key items',
    weapon: 'Weapons',
    armor: 'Equipment',
    food: 'Food slots',
    drop_slot: 'Drop slots',
  };

  // Single attached player is the common case; don't make them click the only row.
  $effect(() => {
    if (attachedPlayers.length > 0 && selectedUid === null) {
      select(attachedPlayers[0]);
    }
    // A detached player must not leave a stale inventory on screen.
    if (selectedUid !== null && !attachedPlayers.includes(selectedUid)) {
      selectedUid = null;
      inventory = null;
    }
  });

  async function select(uid: string) {
    selectedUid = uid;
    inventory = null;
    error = null;
    loading = true;
    try {
      inventory = await client.playerInventory(uid);
    } catch (e) {
      error = e as SaveError;
    } finally {
      loading = false;
    }
  }

  const totalItems = $derived(
    inventory?.containers.reduce(
      (sum, c) => sum + c.slots.reduce((n, s) => n + s.count, 0),
      0,
    ) ?? 0,
  );
</script>

{#if attachedPlayers.length === 0}
  <div class="empty">
    <p><strong>No player save attached.</strong></p>
    <p class="muted">
      <code>Level.sav</code> holds every container in the world but doesn't record which
      one is your backpack — that mapping lives in your own player file. Close this save
      and drop <code>Level.sav</code> together with
      <code>Players/&lt;your-uid&gt;.sav</code> to see inventories.
    </p>
  </div>
{:else}
  <div class="layout">
    {#if attachedPlayers.length > 1}
      <ul class="list">
        {#each attachedPlayers as uid (uid)}
          <li>
            <button class:selected={uid === selectedUid} onclick={() => select(uid)}>
              <span class="mono">{uid}</span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="detail">
      {#if error}
        <p class="error"><code>{error.code}</code> {error.message}</p>
      {/if}

      {#if loading || !inventory}
        {#if !error}<p class="muted">Loading…</p>{/if}
      {:else}
        <p class="muted summary">
          <span class="mono">{inventory.player_uid}</span> — {totalItems.toLocaleString()}
          items across {inventory.containers.length} containers
        </p>

        {#each inventory.containers as container (container.id)}
          <section>
            <h4>
              {KIND_LABELS[container.kind] ?? container.kind}
              <span class="cap">{container.slots.length}/{container.slot_count}</span>
            </h4>

            {#if container.missing}
              <p class="warn">
                This container is referenced by the player save but has no entry in
                <code>Level.sav</code>. That's normal if it was never used — but it also
                happens when the two files come from different worlds.
              </p>
            {:else if container.slots.length === 0}
              <p class="muted small">Empty.</p>
            {:else}
              <div class="tablewrap">
                <table>
                  <thead>
                    <tr><th>Slot</th><th>Item</th><th class="right">Count</th></tr>
                  </thead>
                  <tbody>
                    {#each container.slots as slot (slot.slot_index)}
                      <tr>
                        <td class="mono dim">{slot.slot_index}</td>
                        <td>{slot.static_id ?? '—'}</td>
                        <td class="right">{slot.count.toLocaleString()}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
          </section>
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .empty {
    border: 1px dashed var(--border);
    border-radius: 10px;
    padding: 1.5rem;
  }
  .empty p {
    margin: 0 0 0.5rem;
  }
  .empty p:last-child {
    margin-bottom: 0;
  }
  .layout {
    display: grid;
    grid-template-columns: 1fr;
    gap: 1.5rem;
    align-items: start;
  }
  .layout:has(.list) {
    grid-template-columns: minmax(12rem, 20rem) 1fr;
  }
  @media (max-width: 720px) {
    .layout:has(.list) {
      grid-template-columns: 1fr;
    }
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  .list button {
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
    font-size: 0.8rem;
  }
  .list li:last-child button {
    border-bottom: none;
  }
  .list button:hover {
    background: var(--surface-hover);
  }
  .list button.selected {
    background: var(--accent-soft);
  }
  .summary {
    font-size: 0.85rem;
    margin: 0 0 1.25rem;
  }
  section {
    margin-bottom: 1.5rem;
  }
  h4 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
  }
  .cap {
    color: var(--muted);
    font-size: 0.78rem;
    font-weight: 400;
  }
  .tablewrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: 10px;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.875rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.3rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  th {
    color: var(--muted);
    font-weight: 500;
    font-size: 0.78rem;
  }
  .right {
    text-align: right;
  }
  .dim {
    color: var(--muted);
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.85em;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 0.85rem;
  }
  .warn {
    color: var(--warn);
    font-size: 0.85rem;
    margin: 0;
  }
  .error {
    color: var(--danger);
    font-size: 0.9rem;
  }
  code {
    background: var(--code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
  }
</style>
