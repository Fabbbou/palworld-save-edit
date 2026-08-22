<script lang="ts">
  /** Screen 2: guild list and rename — the first screen that actually edits. */
  import type { GuildDetail, GuildSummary, SaveError } from '../save-types';
  import type { SaveClient } from '../worker/client';

  let {
    client,
    guilds,
    onedited,
  }: {
    client: SaveClient;
    guilds: GuildSummary[];
    /** Fired after a successful rename so the parent can refresh and mark the save dirty. */
    onedited: () => void;
  } = $props();

  let selectedId = $state<string | null>(null);
  let detail = $state<GuildDetail | null>(null);
  let draftName = $state('');
  let saving = $state(false);
  let error = $state<SaveError | null>(null);

  /** Named guilds can be renamed; an Organization has no name field at all. */
  const isNamed = $derived(
    detail?.summary.group_type === 'EPalGroupType::Guild' ||
      detail?.summary.group_type === 'EPalGroupType::IndependentGuild',
  );
  const dirty = $derived(detail !== null && draftName !== detail.summary.name);

  async function select(id: string) {
    selectedId = id;
    detail = null;
    error = null;
    try {
      detail = await client.guild(id);
      draftName = detail.summary.name;
    } catch (e) {
      error = e as SaveError;
    }
  }

  async function rename() {
    if (!selectedId || !dirty) return;
    saving = true;
    error = null;
    try {
      await client.setGuildName(selectedId, draftName);
      detail = await client.guild(selectedId);
      draftName = detail.summary.name;
      onedited();
    } catch (e) {
      error = e as SaveError;
    } finally {
      saving = false;
    }
  }

  function shortType(t: string) {
    return t.replace('EPalGroupType::', '');
  }

  function ticksToDate(ticks: string): string {
    // Unreal FDateTime: 100ns intervals since 0001-01-01. Do the arithmetic in
    // BigInt — these values are far past what a JS number holds exactly.
    try {
      const epochOffset = 621355968000000000n; // ticks from 0001-01-01 to 1970-01-01
      const ms = (BigInt(ticks) - epochOffset) / 10000n;
      const date = new Date(Number(ms));
      return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
    } catch {
      return '—';
    }
  }
</script>

<div class="layout">
  <ul class="list">
    {#each guilds as guild (guild.id)}
      <li>
        <button class:selected={guild.id === selectedId} onclick={() => select(guild.id)}>
          <span class="name">{guild.name || '(unnamed)'}</span>
          <span class="meta">
            {shortType(guild.group_type)} · {guild.member_count}
            {guild.member_count === 1 ? 'member' : 'members'} · {guild.pal_count} pals
          </span>
        </button>
      </li>
    {/each}
  </ul>

  <div class="detail">
    {#if error}
      <p class="error"><code>{error.code}</code> {error.message}</p>
    {/if}

    {#if !selectedId}
      <p class="muted">Select a guild to view its members and rename it.</p>
    {:else if !detail}
      <p class="muted">Loading…</p>
    {:else}
      <h3>{detail.summary.name || '(unnamed)'}</h3>
      <p class="muted mono">{detail.summary.id}</p>

      {#if isNamed}
        <label class="rename">
          <span>Guild name</span>
          <input
            type="text"
            bind:value={draftName}
            disabled={saving}
            onkeydown={(e) => e.key === 'Enter' && rename()}
          />
        </label>
        <button class="primary" onclick={rename} disabled={!dirty || saving}>
          {saving ? 'Renaming…' : 'Rename'}
        </button>
      {:else}
        <p class="muted">
          A {shortType(detail.summary.group_type)} has no name field — nothing to edit here.
        </p>
      {/if}

      <h4>Base camp level: {detail.summary.base_camp_level}</h4>

      {#if detail.members.length > 0}
        <h4>Members ({detail.members.length})</h4>
        <table>
          <thead>
            <tr><th>Name</th><th>Last online</th>{#if detail.members.some((m) => m.role !== null)}<th>Role</th>{/if}</tr>
          </thead>
          <tbody>
            {#each detail.members as member (member.player_uid)}
              <tr>
                <td>
                  {member.player_name || '(unnamed)'}
                  {#if member.player_uid === detail.admin_player_uid}<span class="badge">admin</span>{/if}
                </td>
                <td class="mono">{ticksToDate(member.last_online_real_time)}</td>
                {#if detail.members.some((m) => m.role !== null)}<td>{member.role ?? '—'}</td>{/if}
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {/if}
  </div>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: minmax(14rem, 22rem) 1fr;
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
    margin: 1.25rem 0 0.5rem;
    font-size: 0.95rem;
  }
  .rename {
    display: block;
    margin: 1.25rem 0 0.6rem;
  }
  .rename span {
    display: block;
    font-size: 0.85rem;
    color: var(--muted);
    margin-bottom: 0.3rem;
  }
  input[type='text'] {
    width: 100%;
    max-width: 28rem;
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: inherit;
    font: inherit;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: 0.9rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.35rem 0.6rem;
    border-bottom: 1px solid var(--border);
  }
  th {
    color: var(--muted);
    font-weight: 500;
    font-size: 0.8rem;
  }
  .badge {
    font-size: 0.7rem;
    background: var(--accent-soft);
    color: var(--accent);
    padding: 0.1em 0.4em;
    border-radius: 4px;
    margin-left: 0.4rem;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.85em;
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
