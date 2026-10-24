<!--
  Add and remove tags on one or more images. Changes are saved straight away.

  Type a tag and press Enter (or comma) to add it; existing tags are suggested as you type.
  ↑/↓ pick a suggestion, Tab completes it. Enter on an empty box, or Esc, closes the editor.
  With several images selected, a tag only some of them have is shown faded with a count,
  and clicking it adds it to the rest.
-->
<script lang="ts">
  import type { Tag } from "./api";

  interface Props {
    /** Every tag in the workspace. */
    allTags: Tag[];
    /** How many images in the workspace use each tag (to suggest the most used first). */
    usage: Map<number, number>;
    /** How many images are being tagged. */
    targetCount: number;
    /** For each tag, how many of those images already have it. */
    applied: Map<number, number>;
    /** What is being tagged, e.g. "3 images" or a file name. */
    subject: string;
    onadd: (name: string) => Promise<void>;
    onremove: (tagId: number) => Promise<void>;
    onclose: () => void;
  }

  let { allTags, usage, targetCount, applied, subject, onadd, onremove, onclose }: Props = $props();

  const MAX_SUGGESTIONS = 8;

  let text = $state("");
  /** Index into `options` picked with the arrow keys, or -1. */
  let highlighted = $state(-1);
  let error = $state<string | null>(null);

  const chips = $derived(allTags.filter((t) => applied.has(t.id)));

  type Option = { kind: "tag"; tag: Tag } | { kind: "create"; name: string };

  const options = $derived.by((): Option[] => {
    const query = text.trim().toLowerCase();
    // Tags every image already has aren't worth suggesting.
    const candidates = allTags.filter((t) => (applied.get(t.id) ?? 0) < targetCount);
    const used = (t: Tag) => usage.get(t.id) ?? 0;
    let matches: Tag[];
    if (query) {
      matches = candidates
        .filter((t) => t.name.toLowerCase().includes(query))
        .sort((a, b) => {
          const aStarts = a.name.toLowerCase().startsWith(query);
          const bStarts = b.name.toLowerCase().startsWith(query);
          return Number(bStarts) - Number(aStarts) || used(b) - used(a) || a.name.localeCompare(b.name);
        });
    } else {
      matches = [...candidates].sort((a, b) => used(b) - used(a) || a.name.localeCompare(b.name));
    }
    const list: Option[] = matches.slice(0, MAX_SUGGESTIONS).map((tag) => ({ kind: "tag", tag }));
    const exists = allTags.some((t) => t.name.toLowerCase() === query);
    if (query && !exists) list.push({ kind: "create", name: text.trim().replace(/\s+/g, " ") });
    return list;
  });

  // Typing changes the suggestions, so start picking from the top again.
  $effect(() => {
    void text;
    highlighted = -1;
  });

  async function add(name: string) {
    name = name.trim();
    if (!name) return;
    error = null;
    try {
      await onadd(name);
      text = "";
    } catch (e) {
      error = String(e);
    }
  }

  async function remove(tag: Tag) {
    error = null;
    try {
      await onremove(tag.id);
    } catch (e) {
      error = String(e);
    }
  }

  function pick(option: Option) {
    add(option.kind === "tag" ? option.tag.name : option.name);
  }

  function onkeydown(e: KeyboardEvent) {
    switch (e.key) {
      case "Enter":
        e.preventDefault();
        if (highlighted >= 0 && options[highlighted]) pick(options[highlighted]);
        else if (text.trim()) add(text);
        else onclose();
        break;
      case ",":
        e.preventDefault();
        add(text);
        break;
      case "ArrowDown":
        e.preventDefault();
        if (options.length) highlighted = (highlighted + 1) % options.length;
        break;
      case "ArrowUp":
        e.preventDefault();
        if (options.length) highlighted = highlighted <= 0 ? options.length - 1 : highlighted - 1;
        break;
      case "Tab": {
        const option = options[highlighted >= 0 ? highlighted : 0];
        if (option?.kind === "tag" && text.trim()) {
          e.preventDefault();
          text = option.tag.name;
        }
        break;
      }
      case "Escape":
        e.preventDefault();
        // The first Esc clears what you typed, the next one closes.
        if (text) text = "";
        else onclose();
        break;
    }
    // Keep keys from also reaching the grid or viewer underneath.
    e.stopPropagation();
  }

  function focusInput(input: HTMLInputElement) {
    input.focus();
  }
</script>

<div class="backdrop" role="presentation" onpointerdown={onclose}></div>

<div class="editor" role="dialog" aria-label="Edit tags">
  <header>
    <h2>Tags <span class="subject">· {subject}</span></h2>
    <button class="close" onclick={onclose} aria-label="Close" title="Close (Esc)">✕</button>
  </header>

  {#if chips.length}
    <div class="chips">
      {#each chips as tag (tag.id)}
        {@const count = applied.get(tag.id) ?? 0}
        {@const partial = count < targetCount}
        <span class="chip" class:partial>
          {#if partial}
            <button class="chip-name" onclick={() => add(tag.name)} title="Only on {count} of {targetCount}. Click to add it to all.">
              {tag.name} <span class="count">{count}/{targetCount}</span>
            </button>
          {:else}
            <span class="chip-name">{tag.name}</span>
          {/if}
          <button
            class="chip-remove"
            onclick={() => remove(tag)}
            aria-label="Remove {tag.name}"
            title={targetCount > 1 ? `Remove from all ${targetCount}` : "Remove"}>✕</button
          >
        </span>
      {/each}
    </div>
  {:else}
    <p class="none">{targetCount > 1 ? "None of these images have tags yet." : "No tags yet."}</p>
  {/if}

  <input
    type="text"
    placeholder={chips.length ? "Add another tag…" : "Add a tag…"}
    bind:value={text}
    {onkeydown}
    {@attach focusInput}
    maxlength="60"
    spellcheck="false"
    autocomplete="off"
    aria-label="Tag name"
  />

  {#if error}
    <p class="error">{error}</p>
  {/if}

  {#if options.length}
    <ul class="options" role="listbox" aria-label="Suggestions">
      {#each options as option, i (option.kind === "tag" ? option.tag.id : "create")}
        <li role="option" aria-selected={i === highlighted}>
          <button class:highlighted={i === highlighted} onclick={() => pick(option)} tabindex="-1">
            {#if option.kind === "tag"}
              <span>{option.tag.name}</span>
              <span class="count">{usage.get(option.tag.id) ?? 0}</span>
            {:else}
              <span>Create “{option.name}”</span>
            {/if}
          </button>
        </li>
      {/each}
    </ul>
  {/if}

  <footer>Enter to add · Tab to complete · Esc to close</footer>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 39;
    background: rgba(0, 0, 0, 0.35);
  }
  .editor {
    position: fixed;
    z-index: 40;
    top: 72px;
    left: 50%;
    transform: translateX(-50%);
    width: min(460px, calc(100vw - 32px));
    padding: 14px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.5);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .subject {
    font-weight: 400;
    color: var(--text-dim);
  }
  .close {
    padding: 2px 8px;
    color: var(--text-dim);
    background: transparent;
    border: none;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 10px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    border-radius: 999px;
    background: #24406e;
    border: 1px solid #3a63a8;
  }
  .chip.partial {
    background: transparent;
    border-style: dashed;
  }
  .chip-name {
    padding: 3px 4px 3px 11px;
    font-size: 13px;
    color: var(--text);
    background: none;
    border: none;
  }
  button.chip-name:hover {
    background: none;
    text-decoration: underline;
  }
  .chip-remove {
    padding: 3px 9px 3px 5px;
    font-size: 11px;
    color: var(--text-dim);
    background: none;
    border: none;
    border-radius: 0 999px 999px 0;
  }
  .chip-remove:hover {
    color: #fff;
    background: none;
  }
  .count {
    font-size: 11px;
    color: var(--text-dim);
  }
  .none {
    margin: 0 0 10px;
    font-size: 13px;
    color: var(--text-dim);
  }
  input {
    width: 100%;
    padding: 8px 10px;
    font: inherit;
    color: var(--text);
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    outline: none;
    user-select: text;
  }
  input:focus {
    border-color: var(--accent);
  }
  .error {
    margin: 8px 0 0;
    font-size: 13px;
    color: #ff9b9b;
  }
  .options {
    margin: 6px 0 0;
    padding: 0;
    list-style: none;
    max-height: 260px;
    overflow-y: auto;
  }
  .options button {
    display: flex;
    justify-content: space-between;
    width: 100%;
    padding: 5px 10px;
    text-align: left;
    background: transparent;
    border: none;
  }
  .options button:hover,
  .options button.highlighted {
    background: var(--button-hover);
  }
  footer {
    margin-top: 10px;
    font-size: 11px;
    color: var(--text-dim);
  }
</style>
