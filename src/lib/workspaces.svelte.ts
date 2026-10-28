// The list of workspaces, kept in sync with the library database.

import {
  addWorkspace,
  listWorkspaces,
  removeWorkspace,
  renameWorkspace,
  setWorkspaceColor,
  setWorkspaceFolder,
  setWorkspaceRecursive,
  type Workspace,
} from "./api";

class Workspaces {
  list = $state<Workspace[]>([]);

  async load() {
    this.list = await listWorkspaces();
  }

  /** Adds a folder as a workspace (or finds the existing one for that folder) and returns it. */
  async add(path: string): Promise<Workspace> {
    const workspace = await addWorkspace(path);
    if (!this.list.some((w) => w.id === workspace.id)) this.list = [...this.list, workspace];
    return workspace;
  }

  async rename(id: number, name: string) {
    this.replace(await renameWorkspace(id, name));
  }

  async setColor(id: number, color: string | null) {
    this.replace(await setWorkspaceColor(id, color));
  }

  async setRecursive(id: number, recursive: boolean) {
    this.replace(await setWorkspaceRecursive(id, recursive));
  }

  async setFolder(id: number, path: string) {
    this.replace(await setWorkspaceFolder(id, path));
  }

  async remove(id: number) {
    await removeWorkspace(id);
    this.list = this.list.filter((w) => w.id !== id);
  }

  private replace(updated: Workspace) {
    this.list = this.list.map((w) => (w.id === updated.id ? updated : w));
  }
}

export const workspaces = new Workspaces();
