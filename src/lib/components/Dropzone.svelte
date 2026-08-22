<script lang="ts">
  /**
   * File intake. Drag-and-drop plus a normal file input — no File System Access API
   * here, because opening is the safe half. In-place *writing* is what needs
   * `showDirectoryPicker`, and that isn't implemented yet (see the note in App.svelte).
   */
  let { onfile, busy = false }: { onfile: (file: File) => void; busy?: boolean } = $props();

  let dragging = $state(false);
  let inputEl: HTMLInputElement;

  function pick(list: FileList | null | undefined) {
    const file = list?.[0];
    if (file) onfile(file);
  }

  function onDrop(event: DragEvent) {
    event.preventDefault();
    dragging = false;
    if (busy) return;
    pick(event.dataTransfer?.files);
  }
</script>

<div
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
    hidden
    onchange={(e) => pick((e.currentTarget as HTMLInputElement).files)}
  />
  {#if busy}
    <p class="headline">Opening…</p>
  {:else}
    <p class="headline">Drop a <code>.sav</code> file here</p>
    <p class="sub">or click to browse — <code>Level.sav</code>, <code>LevelMeta.sav</code>, a player save…</p>
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
