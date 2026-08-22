<script lang="ts">
  /** Screen 1: what the save is. Works with zero RawData decoders, so it stays
   *  useful even on a save whose game version we don't fully understand yet. */
  import type { Diagnostics, SaveSummary } from '../save-types';

  let { summary, diagnostics }: { summary: SaveSummary; diagnostics: Diagnostics | null } = $props();

  const rows = $derived([
    ['Save class', summary.save_game_type],
    ['Engine version', summary.engine_version],
    ['Save game version', String(summary.save_game_version)],
    ['Container', summary.container.format === 'PlM' ? 'PlM (Oodle Mermaid)' : 'PlZ (zlib)'],
    ['Game Pass wrapper', summary.container.was_cnk_wrapped ? 'yes (CNK)' : 'no'],
    ['Decompressed size', `${summary.gvas_len.toLocaleString()} bytes`],
    ['Top-level properties', String(summary.top_level_property_count)],
  ] as const);
</script>

<section>
  <dl>
    {#each rows as [label, value] (label)}
      <div class="row">
        <dt>{label}</dt>
        <dd>{value}</dd>
      </div>
    {/each}
  </dl>

  {#if diagnostics}
    <h3>Compatibility</h3>
    {#if diagnostics.warnings.length === 0}
      <p class="clean">Parsed cleanly — no unknown paths or failed decoders.</p>
    {:else}
      <ul class="warnings">
        {#each diagnostics.warnings as warning (warning)}
          <li><code>{warning}</code></li>
        {/each}
      </ul>
      <p class="muted">
        A warning here means part of the save didn't decode and was left as opaque
        bytes. That region is preserved untouched on export, but it can't be edited.
      </p>
    {/if}
  {/if}
</section>

<style>
  dl {
    margin: 0;
    border: 1px solid var(--border);
    border-radius: 10px;
    overflow: hidden;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(10rem, 16rem) 1fr;
    gap: 1rem;
    padding: 0.6rem 1rem;
    border-bottom: 1px solid var(--border);
  }
  .row:last-child {
    border-bottom: none;
  }
  dt {
    color: var(--muted);
    font-size: 0.9rem;
  }
  dd {
    margin: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.9rem;
    overflow-wrap: anywhere;
  }
  h3 {
    margin: 1.75rem 0 0.6rem;
    font-size: 1rem;
  }
  .clean {
    margin: 0;
    color: var(--ok);
    font-size: 0.9rem;
  }
  .warnings {
    margin: 0 0 0.6rem;
    padding-left: 1.2rem;
  }
  .muted {
    color: var(--muted);
    font-size: 0.85rem;
    margin: 0;
  }
  code {
    background: var(--code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
    font-size: 0.85em;
  }
</style>
