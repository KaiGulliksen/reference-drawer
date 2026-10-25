// The tags of the active workspace, kept in sync with the library database.

import { addTag, deleteTag, getTags, removeTag, renameTag, type Tag, type TagData } from "./api";
import { pathKey } from "./platform";

/** A tag as listed in the sidebar. */
export interface TagItem {
  id: number;
  name: string;
  /** How many images in the current view have this tag. */
  count: number;
}

const NO_TAGS: readonly number[] = [];

const byName = (a: Tag, b: Tag) => a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });

class Tags {
  /** The workspace these tags belong to. */
  workspaceId: number | null = null;
  /** Every tag in the workspace, sorted by name. */
  list = $state.raw<Tag[]>([]);
  /** Tag ids per image, keyed by `pathKey` (which ignores capitals where the system does). Replaced on every change. */
  private byImage = $state.raw(new Map<string, readonly number[]>());
  readonly names = $derived(new Map(this.list.map((t) => [t.id, t.name])));

  // Changes are sent one at a time, in order, so quick successive edits can't overtake each other.
  private queue: Promise<unknown> = Promise.resolve();

  async load(workspaceId: number) {
    this.workspaceId = workspaceId;
    this.list = [];
    this.byImage = new Map();
    const data = await getTags(workspaceId);
    if (this.workspaceId === workspaceId) this.apply(data);
  }

  clear() {
    this.workspaceId = null;
    this.list = [];
    this.byImage = new Map();
  }

  idsOf(path: string): readonly number[] {
    return this.byImage.get(pathKey(path)) ?? NO_TAGS;
  }

  namesOf(path: string): string[] {
    return this.idsOf(path)
      .map((id) => this.names.get(id) ?? "")
      .filter(Boolean)
      .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));
  }

  /** Adds the tag called `name` (created if new) to every image in `paths`. */
  add(paths: string[], name: string) {
    return this.run(async (workspaceId) => {
      const tag = await addTag(workspaceId, paths, name);
      if (workspaceId !== this.workspaceId) return;
      if (!this.list.some((t) => t.id === tag.id)) this.list = [...this.list, tag].sort(byName);
      const map = new Map(this.byImage);
      for (const path of paths) {
        const key = pathKey(path);
        const ids = map.get(key) ?? NO_TAGS;
        if (!ids.includes(tag.id)) map.set(key, [...ids, tag.id]);
      }
      this.byImage = map;
    });
  }

  /** Removes a tag from every image in `paths`. */
  remove(paths: string[], tagId: number) {
    return this.run(async (workspaceId) => {
      const unused = await removeTag(workspaceId, paths, tagId);
      if (workspaceId !== this.workspaceId) return;
      const map = new Map(this.byImage);
      for (const path of paths) {
        const key = pathKey(path);
        const ids = (map.get(key) ?? NO_TAGS).filter((id) => id !== tagId);
        if (ids.length) map.set(key, ids);
        else map.delete(key);
      }
      this.byImage = map;
      if (unused) this.list = this.list.filter((t) => t.id !== tagId);
    });
  }

  rename(tagId: number, name: string) {
    return this.run(async (workspaceId) => {
      const data = await renameTag(workspaceId, tagId, name);
      if (workspaceId === this.workspaceId) this.apply(data);
    });
  }

  delete(tagId: number) {
    return this.run(async (workspaceId) => {
      await deleteTag(workspaceId, tagId);
      if (workspaceId !== this.workspaceId) return;
      this.list = this.list.filter((t) => t.id !== tagId);
      const map = new Map<string, readonly number[]>();
      for (const [key, ids] of this.byImage) {
        const rest = ids.filter((id) => id !== tagId);
        if (rest.length) map.set(key, rest);
      }
      this.byImage = map;
    });
  }

  private apply(data: TagData) {
    this.list = [...data.tags].sort(byName);
    this.byImage = new Map(data.images.map((i) => [pathKey(i.path), i.tags]));
  }

  private run(change: (workspaceId: number) => Promise<void>): Promise<void> {
    const workspaceId = this.workspaceId;
    if (workspaceId === null) return Promise.reject(new Error("No workspace is open."));
    const done = this.queue.then(() => change(workspaceId));
    this.queue = done.catch(() => {});
    return done;
  }
}

export const tags = new Tags();
