<script lang="ts">
  /**
   * Screen 4: a player's Pals and inventories. Read-only.
   *
   * Needs two files — the ids come from the player's own save, the contents from
   * Level.sav — so this screen is empty until a `Players/*.sav` is attached. That
   * empty state has to explain itself, otherwise the tab just looks broken.
   *
   * Pals live here rather than in their own tab because they need exactly the same
   * pairing, and the party and Pal box are containers like any other. The Players tab
   * answers "which Pals does this player have"; this one answers "where are they".
   */
  import type {
    ContainerView,
    PalContainerView,
    PlayerInventory,
    PlayerPalStorage,
    SaveError,
  } from '../save-types';
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
  let palStorage = $state<PlayerPalStorage | null>(null);
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

  const PAL_KIND_LABELS: Record<PalContainerView['kind'], string> = {
    party: 'Party',
    storage: 'Pal box',
  };

  /** `Lamball (Bob)` when nicknamed, `Lamball` when not, the raw id as a last resort. */
  function palName(slot: PlayerPalStorage['containers'][number]['slots'][number]): string {
    if (!slot.pal) return slot.instance_id;
    const species = slot.pal.character_id ?? 'Unknown';
    return slot.pal.nickname ? `${species} (${slot.pal.nickname})` : species;
  }

  function ivs(pal: NonNullable<PlayerPalStorage['containers'][number]['slots'][number]['pal']>) {
    const parts = [pal.talent_hp, pal.talent_shot, pal.talent_defense];
    return parts.every((p) => p === null) ? null : parts.map((p) => p ?? '?').join('/');
  }

  // Single attached player is the common case; don't make them click the only row.
  $effect(() => {
    if (attachedPlayers.length > 0 && selectedUid === null) {
      select(attachedPlayers[0]);
    }
    // A detached player must not leave a stale inventory on screen.
    if (selectedUid !== null && !attachedPlayers.includes(selectedUid)) {
      selectedUid = null;
      inventory = null;
      palStorage = null;
    }
  });

  async function select(uid: string) {
    selectedUid = uid;
    inventory = null;
    palStorage = null;
    error = null;
    loading = true;
    try {
      // Both halves of the screen need the same file pair, so fetch them together
      // rather than making the Pal box pop in after the items.
      [inventory, palStorage] = await Promise.all([
        client.playerInventory(uid),
        client.playerPalStorage(uid),
      ]);
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

  const totalPals = $derived(
    palStorage?.containers.reduce((sum, c) => sum + c.slots.length, 0) ?? 0,
  );
</script>

{#if attachedPlayers.length === 0}
  <div class="empty" data-testid="inventory-empty">
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
        <p class="muted summary" data-testid="inventory-summary">
          <span class="mono">{inventory.player_uid}</span> — {totalPals.toLocaleString()} Pals
          and {totalItems.toLocaleString()} items across
          {inventory.containers.length} containers
        </p>

        {#if palStorage}
          {#each palStorage.containers as container (container.id)}
            <section data-testid="pal-container-{container.kind}">
              <h4>
                {PAL_KIND_LABELS[container.kind] ?? container.kind}
                <span class="cap">{container.slots.length}/{container.slot_count}</span>
              </h4>

              {#if container.missing}
                <p class="warn">
                  This Pal container is referenced by the player save but has no entry in
                  <code>Level.sav</code>. That also happens when the two files come from
                  different worlds.
                </p>
              {:else if container.slots.length === 0}
                <p class="muted small">Empty.</p>
              {:else}
                <div class="tablewrap">
                  <table>
                    <thead>
                      <tr>
                        <th>Slot</th><th>Pal</th><th class="right">Lv</th>
                        <th class="right">IVs</th><th>Gender</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each container.slots as slot (slot.instance_id)}
                        <tr>
                          <td class="mono dim">{slot.slot_index}</td>
                          <td>
                            {palName(slot)}
                            {#if slot.pal?.is_rare}<span class="badge rare">rare</span>{/if}
                            {#if !slot.pal}
                              <span class="warn small">
                                — no matching Pal in this world
                              </span>
                            {/if}
                          </td>
                          <td class="right">{slot.pal?.level ?? '—'}</td>
                          <td class="right mono">{(slot.pal && ivs(slot.pal)) ?? '—'}</td>
                          <td class="dim">{slot.pal?.gender ?? '—'}</td>
                        </tr>
                      {/each}
                    </tbody>
                  </table>
                </div>
              {/if}
            </section>
          {/each}
        {/if}

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
                    <tr>
                      <th>Slot</th><th>Item</th><th class="right">Count</th>
                      <th>State</th>
                    </tr>
                  </thead>
                  <tbody>
                    {#each container.slots as slot (slot.slot_index)}
                      <tr>
                        <td class="mono dim">{slot.slot_index}</td>
                        <td>{slot.static_id ?? '—'}</td>
                        <td class="right">{slot.count.toLocaleString()}</td>
                        <td class="dim small">
                          <!-- Absent for most items: a stack of Wood has no
                               per-instance state, which is not the same as a
                               failed lookup. Blank is the honest rendering. -->
                          {#if slot.egg_character_id}
                            contains {slot.egg_character_id}
                          {:else}
                            {#if slot.durability !== null}
                              {slot.durability.toLocaleString()} dur
                            {/if}
                            {#if slot.remaining_bullets}
                              · {slot.remaining_bullets} loaded
                            {/if}
                            {#if slot.ammo_static_id}
                              · {slot.ammo_static_id}
                            {/if}
                          {/if}
                        </td>
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
  /* Same badge as the Players screen, so a rare Pal reads identically wherever it
     shows up. */
  .badge {
    display: inline-block;
    font-size: 0.7rem;
    background: var(--surface-hover);
    border: 1px solid var(--border);
    padding: 0.05em 0.4em;
    border-radius: 4px;
    margin: 0.1em 0.2em 0.1em 0;
  }
  .badge.rare {
    background: var(--warn-soft);
    color: var(--warn);
    border-color: var(--warn-border);
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
