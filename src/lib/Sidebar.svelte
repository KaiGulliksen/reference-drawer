<!--
  The left sidebar: the workspaces, and the tags used in the open workspace.
-->
<script lang="ts">
  import type { Workspace } from "./api";
  import { hasPrimaryModifier, primaryKeyLabel, shortcutLabel } from "./platform";
  import type { TagItem } from "./tags.svelte";

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

    /** Tags used by images in the open workspace, sorted by name. */
    tagItems: TagItem[];
    untaggedCount: number;
    /** Tags the grid is filtered by. */
    filterTags: Set<number>;
    filterUntagged: boolean;
    /** Text typed in the tag search box. */
    tagSearch: string;
    /** The tag whose name is being edited, if any. */
    renamingTagId: number | null;
    /** Click on a tag (or on "Untagged"). `add` is true for Ctrl-click (Cmd on a Mac), to combine tags. */
    ontagclick: (id: number | "untagged", add: boolean) => void;
    ontagrename: (id: number, name: string) => void;
    ontagmenu: (tag: TagItem, x: number, y: number) => void;
  }

  let {
    workspaces,
    activeId,
    renamingId = $bindable(),
    missing,
    onselect,
    onadd,
    onrename,
    onmenu,
    tagItems,
    untaggedCount,
    filterTags,
    filterUntagged,
    tagSearch = $bindable(),
    renamingTagId = $bindable(),
    ontagclick,
    ontagrename,
    ontagmenu,
  }: Props = $props();

  const shownTags = $derived.by(() => {
    const query = tagSearch.trim().toLowerCase();
    return query ? tagItems.filter((t) => t.name.toLowerCase().includes(query)) : tagItems;
  });

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

  function finishTagRename(tag: TagItem, input: HTMLInputElement, save: boolean) {
    if (renamingTagId !== tag.id) return;
    renamingTagId = null;
    const name = input.value.trim();
    if (save && name && name !== tag.name) ontagrename(tag.id, name);
  }

  function onSearchKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && shownTags.length > 0) {
      // Enter shows the images with the best-matching tag, and hands the keyboard back to
      // the grid so you can carry on with the arrow keys or the tag shortcut.
      e.preventDefault();
      ontagclick(shownTags[0].id, hasPrimaryModifier(e));
      e.currentTarget instanceof HTMLInputElement && e.currentTarget.blur();
    } else if (e.key === "Escape") {
      e.preventDefault();
      tagSearch = "";
      e.currentTarget instanceof HTMLInputElement && e.currentTarget.blur();
    }
  }

  function shortcutHint(i: number) {
    return i < 9 ? `\n${shortcutLabel(String(i + 1))} to switch here` : "";
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

  {#if activeId !== null && !missing.has(activeId)}
    <section class="tags">
      <header>
        <h2>Tags</h2>
      </header>

      <div class="search">
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
          <circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.4" />
          <path d="M10.5 10.5l3.5 3.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
        </svg>
        <input
          type="search"
          placeholder="Search tags…"
          bind:value={tagSearch}
          onkeydown={onSearchKeydown}
          spellcheck="false"
          autocomplete="off"
          aria-label="Search tags"
        />
      </div>

      <ul class="tag-list">
        {#if !tagSearch.trim()}
          <li class:active={filterUntagged}>
            <button
              class="item tag"
              onclick={() => ontagclick("untagged", false)}
              title="Show images that have no tags yet"
            >
              <span class="name untagged">Untagged</span>
              <span class="count">{untaggedCount.toLocaleString()}</span>
            </button>
          </li>
        {/if}
        {#each shownTags as tag (tag.id)}
          <li class:active={filterTags.has(tag.id)}>
            {#if renamingTagId === tag.id}
              <input
                class="rename"
                value={tag.name}
                aria-label="Tag name"
                maxlength="60"
                {@attach focusAndSelect}
                onkeydown={(e) => {
                  if (e.key === "Enter") finishTagRename(tag, e.currentTarget, true);
                  else if (e.key === "Escape") finishTagRename(tag, e.currentTarget, false);
                }}
                onblur={(e) => finishTagRename(tag, e.currentTarget, true)}
              />
            {:else}
              <button
                class="item tag"
                title={`Show images tagged “${tag.name}”\n${primaryKeyLabel}+click to combine with other tags · double-click to rename`}
                onclick={(e) => ontagclick(tag.id, hasPrimaryModifier(e))}
                ondblclick={() => (renamingTagId = tag.id)}
                oncontextmenu={(e) => {
                  e.preventDefault();
                  ontagmenu(tag, e.clientX, e.clientY);
                }}
              >
                <span class="hash" aria-hidden="true">#</span>
                <span class="name">{tag.name}</span>
                <span class="count">{tag.count.toLocaleString()}</span>
              </button>
            {/if}
          </li>
        {/each}
      </ul>

      {#if tagItems.length === 0}
        <p class="empty">
          No tags yet. Select images and press the tag shortcut, or right-click an image, to add some.
        </p>
      {:else if shownTags.length === 0}
        <p class="empty">No tags match “{tagSearch.trim()}”.</p>
      {/if}
    </section>
  {/if}
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
  section.tags {
    margin-top: 18px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 4px 6px;
    padding: 0 8px;
    color: var(--text-dim);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 5px;
  }
  .search:focus-within {
    border-color: var(--accent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    padding: 5px 0;
    font: inherit;
    font-size: 13px;
    color: var(--text);
    background: transparent;
    border: none;
    outline: none;
    user-select: text;
  }
  .item.tag {
    padding: 4px 8px 4px 12px;
    font-size: 13px;
  }
  .hash {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  li.active .hash {
    color: var(--accent);
  }
  .untagged {
    font-style: italic;
    color: var(--text-dim);
  }
  .count {
    margin-left: auto;
    padding-left: 8px;
    font-size: 12px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
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
