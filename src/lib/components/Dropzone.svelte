<script lang="ts">
  /**
   * File intake. Drag-and-drop plus a normal file input — no File System Access API
   * here, because opening is the safe half. In-place *writing* is what needs
   * `showDirectoryPicker`, and that isn't implemented yet (see the note in App.svelte).
   *
   * Hands back **every** dropped `.sav`. Which one is the level and which are player
   * saves is decided by the caller from each file's save class, not from its name —
   * users rename files, and a wrong guess would attribute one player's inventory to
   * another.
   */
  let { onfiles, busy = false }: { onfiles: (files: File[]) => void; busy?: boolean } = $props();

  let dragging = $state(false);
  let inputEl: HTMLInputElement;

  function pick(list: FileList | null | undefined) {
    const files = Array.from(list ?? []).filter((f) => f.name.toLowerCase().endsWith('.sav'));
    if (files.length > 0) onfiles(files);
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    dragging = false;
    if (busy) return;
    pick(event.dataTransfer?.files);
  }
</script>

<div
  data-testid="dropzone"
  class="dropzone"
  class:dragging
  class:busy
  ondragover={(e) => {
    e.preventDefault();
    dragging = true;
  }}
  ondragleave={() => (dragging = false)}
  ondrop={onDrop}
  role="button"
  tabindex="0"
  onclick={() => !busy && inputEl.click()}
  onkeydown={(e) => {
    if (!busy && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      inputEl.click();
    }
  }}
>
  <input
    bind:this={inputEl}
    type="file"
    accept=".sav"
    multiple
    hidden
    data-testid="file-input"
    onchange={(e) => pick((e.currentTarget as HTMLInputElement).files)}
  />
  {#if busy}
    <p class="headline">Opening…</p>
  {:else}
    <p class="headline">Drop your <code>.sav</code> files here</p>
    <p class="sub">
      or click to browse. Drop <code>Level.sav</code> <em>and</em> your
      <code>Players/*.sav</code> together to see inventories.
    </p>
  {/if}
  <p class="privacy">Nothing is uploaded. The file is read in your browser and never leaves this tab.</p>
</div>

<style>
  .dropzone {
    border: 2px dashed var(--border);
    border-radius: 12px;
    padding: 3rem 2rem;
    text-align: center;
    cursor: pointer;
    background: var(--surface);
    transition: border-color 0.15s, background 0.15s;
  }
  .dropzone:hover:not(.busy),
  .dropzone.dragging {
    border-color: var(--accent);
    background: var(--surface-hover);
  }
  .dropzone.busy {
    cursor: progress;
    opacity: 0.7;
  }
  .headline {
    font-size: 1.15rem;
    margin: 0 0 0.4rem;
  }
  .sub {
    margin: 0;
    color: var(--muted);
    font-size: 0.9rem;
  }
  .privacy {
    margin: 1.5rem 0 0;
    font-size: 0.8rem;
    color: var(--muted);
  }
  code {
    background: var(--code-bg);
    padding: 0.1em 0.35em;
    border-radius: 4px;
    font-size: 0.9em;
  }
</style>
