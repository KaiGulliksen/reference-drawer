// Shared types for right-click menus (see ContextMenu.svelte).

export type MenuItem =
  | {
      label: string;
      action: () => void;
      /** Shown in red, for actions like removing something. */
      danger?: boolean;
      disabled?: boolean;
    }
  | "separator";

export interface MenuRequest {
  x: number;
  y: number;
  items: MenuItem[];
}
