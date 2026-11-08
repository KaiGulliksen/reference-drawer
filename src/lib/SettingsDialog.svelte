<!--
  App settings (what thumbnails show, the tag shortcut, where the library's backups are) and,
  on a second tab, the About page. They share a dialog so Esc and the mouse back button
  close both the same way.
-->
<script lang="ts">
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import { backupsFolder } from "./api";
  import { fileManagerName } from "./platform";
  import { RESERVED_KEYS, keyLabel, settings } from "./settings.svelte";

  interface Props {
    onclose: () => void;
  }

  let { onclose }: Props = $props();

  // The app is only allowed to open this exact address (capabilities/default.json),
  // so if it changes here it has to change there too.
  const WEBSITE = "https://kaigulliksen.com";

  let page = $state<"settings" | "about">("settings");

  function showPage(next: typeof page) {
    page = next;
    // Leaving the page mid-way shouldn't leave it waiting for a key nobody can see it asking for.
    recording = false;
    message = null;
  }

  async function openWebsite() {
    try {
      await openUrl(WEBSITE);
    } catch (e) {
      message = String(e);
    }
  }

  async function showBackups() {
    try {
      await revealItemInDir(await backupsFolder());
    } catch (e) {
      message = String(e);
    }
  }

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

<div class="dialog" role="dialog" aria-modal="true" aria-label="Settings">
  <header>
    <nav class="tabs">
      <button class:current={page === "settings"} aria-current={page === "settings" ? "page" : undefined} onclick={() => showPage("settings")}>Settings</button>
      <button class:current={page === "about"} aria-current={page === "about" ? "page" : undefined} onclick={() => showPage("about")}>About</button>
    </nav>
    <button class="close" onclick={onclose} aria-label="Close" title="Close (Esc)">✕</button>
  </header>

  <!-- Both pages sit in the same spot and the hidden one keeps its space, so the dialog
       doesn't change size and jump around when switching tabs. -->
  <div class="pages">
    <div class:hidden={page !== "settings"}>
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

      <section>
        <h3>Library</h3>
        <div class="row">
          <div>
            <div>Backups</div>
            <div class="hint">Your tags and boards are copied once a day (the last 7 days are kept) and before each upgrade.</div>
          </div>
          <button class="no-wrap" onclick={showBackups} title="Show the backups folder in {fileManagerName}">Show folder</button>
        </div>
      </section>
    </div>

    <div class="about" class:hidden={page !== "about"}>
      <p class="name">Reference Drawer</p>
      <p>An open source project. The source code isn't public yet, but it will be.</p>
      <p>Released under the MIT License by Kai Gulliksen.</p>
      <!-- A button rather than a link: a real link would load the site inside the app's window. -->
      <p><button class="link" onclick={openWebsite} title="Open in your web browser">kaigulliksen.com</button></p>
      {#if message}
        <p class="message">{message}</p>
      {/if}
    </div>
  </div>
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
  .tabs {
    display: flex;
    gap: 16px;
  }
  .tabs button {
    padding: 0;
    font-size: 17px;
    font-weight: 600;
    color: var(--text-dim);
    background: transparent;
    border: none;
  }
  .tabs button:hover,
  .tabs button.current {
    color: var(--text);
  }
  .pages {
    display: grid;
  }
  .pages > div {
    grid-area: 1 / 1;
    min-width: 0;
  }
  .hidden {
    visibility: hidden;
  }
  .about {
    padding-top: 16px;
  }
  .about p {
    margin: 0 0 10px;
  }
  .about .name {
    font-weight: 600;
  }
  .link {
    padding: 0;
    color: var(--accent);
    background: transparent;
    border: none;
    text-decoration: underline;
  }
  .link:hover {
    background: transparent;
    filter: brightness(1.15);
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
  .no-wrap {
    flex-shrink: 0;
    white-space: nowrap;
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
