<!--
  The left sidebar: the list of workspaces. Built in sections so a "Tags"
  section can sit below the workspaces later.
-->
<script lang="ts">
  import type { Workspace } from "./api";

  interface Props {
    workspaces: Workspace[];
    activeId: number | null;
    /** The workspace whose name is being edited, if any. */
    renamingId: number | null;
    /** Workspaces whose folder couldn't be found. */
    missing: Set<number>;
    onselect: (id: number) => void;
    onadd: () => void;
    onrename: (id: number, name: string) => void;
    onmenu: (workspace: Workspace, x: number, y: number) => void;
  }

  let { workspaces, activeId, renamingId = $bindable(), missing, onselect, onadd, onrename, onmenu }: Props = $props();

  function focusAndSelect(input: HTMLInputElement) {
    input.focus();
    input.select();
  }

  function finishRename(workspace: Workspace, input: HTMLInputElement, save: boolean) {
    if (renamingId !== workspace.id) return; // already finished (e.g. Enter, then the blur that follows)
    renamingId = null;
    const name = input.value.trim();
    if (save && name && name !== workspace.name) onrename(workspace.id, name);
  }

  function shortcutHint(i: number) {
    return i < 9 ? `\nCtrl+${i + 1} to switch here` : "";
  }
</script>

<nav class="sidebar" aria-label="Workspaces">
  <section>
    <header>
      <h2>Workspaces</h2>
      <button class="icon" onclick={onadd} title="Add a folder as a new workspace" aria-label="Add workspace">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M8 3v10M3 8h10" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
        </svg>
      </button>
    </header>

    {#if workspaces.length === 0}
      <p class="empty">
        No workspaces yet.<br />
        <button class="link" onclick={onadd}>Add a folder</button> to start one.
      </p>
    {/if}

    <ul>
      {#each workspaces as workspace, i (workspace.id)}
        <li class:active={workspace.id === activeId} class:missing={missing.has(workspace.id)}>
          {#if renamingId === workspace.id}
            <input
              class="rename"
              value={workspace.name}
              aria-label="Workspace name"
              {@attach focusAndSelect}
              onkeydown={(e) => {
                if (e.key === "Enter") finishRename(workspace, e.currentTarget, true);
                else if (e.key === "Escape") finishRename(workspace, e.currentTarget, false);
              }}
              onblur={(e) => finishRename(workspace, e.currentTarget, true)}
            />
          {:else}
            <button
              class="item"
              title="{workspace.path}{shortcutHint(i)}"
              aria-current={workspace.id === activeId ? "page" : undefined}
              onclick={() => onselect(workspace.id)}
              ondblclick={() => (renamingId = workspace.id)}
              oncontextmenu={(e) => {
                e.preventDefault();
                onmenu(workspace, e.clientX, e.clientY);
              }}
            >
              <svg class="folder" viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                <path
                  d="M1.5 4.5a1 1 0 0 1 1-1h3.6l1.4 1.5h6a1 1 0 0 1 1 1v6.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.2"
                />
              </svg>
              <span class="name">{workspace.name}</span>
            </button>
            <button
              class="more"
              title="Workspace options"
              aria-label="Options for {workspace.name}"
              onclick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                onmenu(workspace, r.left, r.bottom + 2);
              }}>⋯</button
            >
          {/if}
        </li>
      {/each}
    </ul>
  </section>
</nav>

<style>
  .sidebar {
    width: 220px;
    flex-shrink: 0;
    overflow-y: auto;
    padding: 8px 6px;
    background: var(--panel);
    border-right: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 4px 6px 8px;
  }
  h2 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    color: var(--text-dim);
    background: transparent;
    border: none;
  }
  .icon:hover {
    color: var(--text);
    background: var(--button-hover);
  }
  .empty {
    margin: 4px 8px;
    font-size: 13px;
    line-height: 1.6;
    color: var(--text-dim);
  }
  .link {
    padding: 0;
    color: var(--accent);
    background: none;
    border: none;
  }
  .link:hover {
    text-decoration: underline;
    background: none;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: 5px;
  }
  li:hover {
    background: var(--button);
  }
  li.active {
    background: var(--button-hover);
  }
  li.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 6px;
    bottom: 6px;
    width: 3px;
    border-radius: 2px;
    background: var(--accent);
  }
  .item {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px 6px 12px;
    text-align: left;
    background: transparent;
    border: none;
  }
  .item:hover {
    background: transparent;
  }
  .folder {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  li.active .folder {
    color: var(--accent);
  }
  li.missing .name {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .more {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    margin-right: 2px;
    padding: 0;
    color: var(--text-dim);
    background: transparent;
    border: none;
    opacity: 0;
  }
  li:hover .more,
  .more:focus-visible {
    opacity: 1;
  }
  .more:hover {
    color: var(--text);
    background: var(--border);
  }
  .rename {
    flex: 1;
    min-width: 0;
    margin: 2px;
    padding: 4px 8px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--accent);
    border-radius: 4px;
    outline: none;
    user-select: text;
  }
</style>
