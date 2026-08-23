<script lang="ts">
  /** Screen 3: players and the Pals they own. Read-only. */
  import type { PalSummary, PlayerDetail, PlayerSummary, SaveError } from '../save-types';
  import type { SaveClient } from '../worker/client';

  let {
    client,
    players,
  }: {
    client: SaveClient;
    players: PlayerSummary[];
  } = $props();

  let selectedUid = $state<string | null>(null);
  let detail = $state<PlayerDetail | null>(null);
  let error = $state<SaveError | null>(null);
  let loading = $state(false);
  let search = $state('');
  let sortKey = $state<'level' | 'name' | 'iv'>('level');

  // Auto-select when there's only one player — the common single-player case, where
  // making someone click the only row is pure friction.
  $effect(() => {
    if (players.length > 0 && selectedUid === null) {
      select(players[0].uid);
    }
  });

  async function select(uid: string) {
    selectedUid = uid;
    detail = null;
    error = null;
    loading = true;
    try {
      detail = await client.player(uid);
    } catch (e) {
      error = e as SaveError;
    } finally {
      loading = false;
    }
  }

  /** Total IVs, the number players actually compare Pals by. */
  function ivTotal(p: PalSummary): number {
    return (p.talent_hp ?? 0) + (p.talent_shot ?? 0) + (p.talent_defense ?? 0);
  }

  const visiblePals = $derived.by(() => {
    if (!detail) return [];
    const needle = search.trim().toLowerCase();
    const filtered = needle
      ? detail.pals.filter(
          (p) =>
            (p.character_id ?? '').toLowerCase().includes(needle) ||
            (p.nickname ?? '').toLowerCase().includes(needle) ||
            p.passive_skills.some((s) => s.toLowerCase().includes(needle)),
        )
      : detail.pals;

    return [...filtered].sort((a, b) => {
      if (sortKey === 'name') {
        return (a.character_id ?? '').localeCompare(b.character_id ?? '');
      }
      if (sortKey === 'iv') return ivTotal(b) - ivTotal(a);
      return (b.level ?? 0) - (a.level ?? 0);
    });
  });

  /** Fixed-point game stats arrive as strings to avoid precision loss; HP is stored
   *  ×1000. Returns null rather than 0 so a missing stat renders as "—". */
  function hp(value: string | null): number | null {
    if (value === null) return null;
    try {
      return Number(BigInt(value) / 1000n);
    } catch {
      return null;
    }
  }

  function num(value: string | null): string {
    if (value === null) return '—';
    try {
      return BigInt(value).toLocaleString();
    } catch {
      return value;
    }
  }
</script>

<div class="layout">
  <ul class="list">
    {#each players as player (player.uid)}
      <li>
        <button class:selected={player.uid === selectedUid} onclick={() => select(player.uid)}>
          <span class="name">{player.nickname ?? '(unnamed)'}</span>
          <span class="meta">
            Level {player.level ?? '—'} · {player.pal_count} pals
          </span>
        </button>
      </li>
    {/each}
  </ul>

  <div class="detail">
    {#if error}
      <p class="error"><code>{error.code}</code> {error.message}</p>
    {/if}

    {#if !selectedUid}
      <p class="muted">Select a player.</p>
    {:else if loading || !detail}
      <p class="muted">Loading…</p>
    {:else}
      <h3>{detail.summary.nickname ?? '(unnamed)'}</h3>
      <p class="muted mono uid" title="Also the filename of this player's Players/*.sav">
        {detail.summary.uid}
      </p>

      <dl class="stats">
        <div><dt>Level</dt><dd>{detail.summary.level ?? '—'}</dd></div>
        <div><dt>Exp</dt><dd>{num(detail.summary.exp)}</dd></div>
        <div><dt>HP</dt><dd>{hp(detail.summary.hp)?.toLocaleString() ?? '—'}</dd></div>
        <div><dt>Shield</dt><dd>{hp(detail.summary.shield_hp)?.toLocaleString() ?? '—'}</dd></div>
        <div>
          <dt>Stomach</dt>
          <dd>{detail.summary.full_stomach?.toFixed(1) ?? '—'}</dd>
        </div>
        <div><dt>Pals owned</dt><dd>{detail.summary.pal_count}</dd></div>
      </dl>

      <div class="palbar">
        <h4>Pals ({visiblePals.length}{search ? ` of ${detail.pals.length}` : ''})</h4>
        <div class="controls">
          <input type="search" placeholder="Filter species, name, passive…" bind:value={search} />
          <select bind:value={sortKey} aria-label="Sort Pals by">
            <option value="level">Level</option>
            <option value="iv">Total IV</option>
            <option value="name">Species</option>
          </select>
        </div>
      </div>

      {#if detail.pals.length === 0}
        <p class="muted">
          This player owns no Pals. Pals assigned to a base camp have no owner recorded
          and aren't listed here.
        </p>
      {:else}
        <div class="tablewrap" data-testid="pals-table">
          <table>
            <thead>
              <tr>
                <th>Species</th>
                <th>Lvl</th>
                <th title="HP / Attack / Defense individual values">IVs</th>
                <th>Friendship</th>
                <th>Passives</th>
              </tr>
            </thead>
            <tbody>
              {#each visiblePals as pal (pal.instance_id)}
                <tr>
                  <td>
                    {pal.character_id ?? '—'}
                    {#if pal.nickname}<span class="nick">“{pal.nickname}”</span>{/if}
                    {#if pal.is_rare}<span class="badge rare">rare</span>{/if}
                    {#if pal.rank}<span class="badge">★{pal.rank}</span>{/if}
                    {#if pal.gender}<span class="gender">{pal.gender === 'Female' ? '♀' : '♂'}</span>{/if}
                  </td>
                  <td>{pal.level ?? '—'}</td>
                  <td class="mono ivs">
                    {pal.talent_hp ?? '—'}/{pal.talent_shot ?? '—'}/{pal.talent_defense ?? '—'}
                  </td>
                  <td>{pal.friendship_point?.toLocaleString() ?? '—'}</td>
                  <td class="passives">
                    {#if pal.passive_skills.length === 0}
                      <span class="muted">—</span>
                    {:else}
                      {#each pal.passive_skills as skill (skill)}
                        <span class="badge">{skill}</span>
                      {/each}
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: minmax(12rem, 18rem) 1fr;
    gap: 1.5rem;
    align-items: start;
  }
  @media (max-width: 720px) {
    .layout {
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
  .name {
    display: block;
  }
  .meta {
    display: block;
    color: var(--muted);
    font-size: 0.8rem;
    margin-top: 0.15rem;
  }
  .detail h3 {
    margin: 0 0 0.2rem;
  }
  .detail h4 {
    margin: 0;
    font-size: 0.95rem;
  }
  .uid {
    font-size: 0.75rem;
    margin: 0 0 1rem;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(7rem, 1fr));
    gap: 0.5rem;
    margin: 0 0 1.5rem;
  }
  .stats div {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.5rem 0.7rem;
  }
  .stats dt {
    color: var(--muted);
    font-size: 0.75rem;
  }
  .stats dd {
    margin: 0.1rem 0 0;
    font-size: 1.05rem;
  }
  .palbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    margin-bottom: 0.6rem;
  }
  .controls {
    display: flex;
    gap: 0.4rem;
  }
  input[type='search'],
  select {
    padding: 0.35rem 0.55rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
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
    padding: 0.35rem 0.6rem;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  th {
    color: var(--muted);
    font-weight: 500;
    font-size: 0.78rem;
    position: sticky;
    top: 0;
    background: var(--surface);
  }
  .ivs {
    white-space: nowrap;
  }
  .passives {
    max-width: 26rem;
  }
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
  .nick {
    color: var(--muted);
    font-size: 0.85em;
  }
  .gender {
    color: var(--muted);
    margin-left: 0.25em;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .muted {
    color: var(--muted);
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
