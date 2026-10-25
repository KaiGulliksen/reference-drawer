<!--
  App settings. For now: the keyboard shortcut that opens the tag editor.
-->
<script lang="ts">
  import { RESERVED_KEYS, keyLabel, settings } from "./settings.svelte";

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  /** True while waiting for you to press the new shortcut key. */
  let recording = $state(false);
  let message = $state<string | null>(null);

  function onkeydown(e: KeyboardEvent) {
    e.stopPropagation();
    if (!recording) {
      if (e.key === "Escape") {
        e.preventDefault();
        onclose();
      }
      return;
    }
    e.preventDefault();
    if (["Shift", "Control", "Alt", "Meta", "AltGraph", "CapsLock"].includes(e.key)) return;
    if (e.key === "Escape") {
      recording = false;
      message = null;
      return;
    }
    if (e.ctrlKey || e.altKey || e.metaKey) {
      message = "Use a single key, without modifier keys like Ctrl, Alt or ⌘.";
      return;
    }
    const key = e.key.toLowerCase();
    if (RESERVED_KEYS[key]) {
      message = `${keyLabel(key)} is already used for ${RESERVED_KEYS[key]}. Try another key.`;
      return;
    }
    settings.tagHotkey = key;
    recording = false;
    message = null;
  }
</script>

<svelte:window {onkeydown} />

<!-- Only ordinary clicks close it here; the mouse's back button is handled by the page. -->
<div class="backdrop" role="presentation" onpointerdown={(e) => e.button <= 2 && onclose()}></div>

<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title">
  <header>
    <h2 id="settings-title">Settings</h2>
    <button class="close" onclick={onclose} aria-label="Close" title="Close (Esc)">✕</button>
  </header>

  <section>
    <h3>Thumbnails</h3>
    <label class="row toggle">
      <div>
        <div>Show tags</div>
        <div class="hint">Each image's tags along the bottom of its thumbnail.</div>
      </div>
      <input type="checkbox" bind:checked={settings.showTileTags} />
    </label>
    <label class="row toggle">
      <div>
        <div>Show file names</div>
        <div class="hint">Each image's file name on its thumbnail. It's always shown in the status bar.</div>
      </div>
      <input type="checkbox" bind:checked={settings.showTileNames} />
    </label>
  </section>

  <section>
    <h3>Keyboard shortcuts</h3>
    <div class="row">
      <div>
        <div>Edit tags</div>
        <div class="hint">Tags the selected images, or the image open in the viewer.</div>
      </div>
      <div class="key-setting">
        {#if recording}
          <span class="key recording">Press a key…</span>
          <button onclick={() => ((recording = false), (message = null))}>Cancel</button>
        {:else}
          <kbd class="key">{keyLabel(settings.tagHotkey)}</kbd>
          <button onclick={() => (recording = true)}>Change</button>
          {#if settings.tagHotkey !== "t"}
            <button onclick={() => (settings.tagHotkey = "t")} title="Back to T">Reset</button>
          {/if}
        {/if}
      </div>
    </div>
    {#if message}
      <p class="message">{message}</p>
    {/if}
  </section>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 39;
    background: rgba(0, 0, 0, 0.45);
  }
  .dialog {
    position: fixed;
    z-index: 40;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(520px, calc(100vw - 32px));
    padding: 16px 18px 20px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  h3 {
    margin: 16px 0 10px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .close {
    padding: 2px 8px;
    color: var(--text-dim);
    background: transparent;
    border: none;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .toggle {
    padding: 4px 0;
    cursor: pointer;
  }
  .toggle + .toggle {
    margin-top: 8px;
  }
  .toggle input {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    accent-color: var(--accent);
    cursor: pointer;
  }
  .hint {
    margin-top: 2px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .key-setting {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .key {
    min-width: 34px;
    padding: 3px 10px;
    text-align: center;
    font-family: inherit;
    font-weight: 600;
    background: var(--bg);
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 5px;
  }
  .key.recording {
    font-weight: 400;
    color: var(--accent);
    border-color: var(--accent);
  }
  .message {
    margin: 10px 0 0;
    font-size: 13px;
    color: #ffc58f;
  }
</style>
