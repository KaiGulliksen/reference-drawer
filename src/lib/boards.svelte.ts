// The boards of the active workspace, kept in sync with the library database.

import { createBoard, deleteBoard, listBoards, renameBoard, setBoardColor, type Board } from "./api";

class Boards {
  workspaceId: number | null = null;
  list = $state<Board[]>([]);

  async load(workspaceId: number) {
    this.workspaceId = workspaceId;
    this.list = [];
    const boards = await listBoards(workspaceId);
    if (this.workspaceId === workspaceId) this.list = boards;
  }

  clear() {
    this.workspaceId = null;
    this.list = [];
  }

  /** Re-reads the list (e.g. after images were added, to update the counts). */
  async refresh() {
    if (this.workspaceId !== null) await this.load(this.workspaceId);
  }

  /** Creates a board with a name that isn't taken yet ("New board", "New board 2", …). */
  async create(name = "New board"): Promise<Board> {
    if (this.workspaceId === null) throw new Error("No workspace is open.");
    const taken = new Set(this.list.map((b) => b.name.toLowerCase()));
    let unique = name;
    for (let n = 2; taken.has(unique.toLowerCase()); n++) unique = `${name} ${n}`;
    const board = await createBoard(this.workspaceId, unique);
    this.list = [...this.list, board];
    return board;
  }

  async rename(id: number, name: string) {
    const board = await renameBoard(id, name);
    this.list = this.list.map((b) => (b.id === id ? board : b));
  }

  async setColor(id: number, color: string | null) {
    const board = await setBoardColor(id, color);
    this.list = this.list.map((b) => (b.id === id ? board : b));
  }

  async remove(id: number) {
    await deleteBoard(id);
    this.list = this.list.filter((b) => b.id !== id);
  }

  setCount(id: number, imageCount: number) {
    this.list = this.list.map((b) => (b.id === id ? { ...b, imageCount } : b));
  }
}

export const boards = new Boards();
