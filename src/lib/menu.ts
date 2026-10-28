// Shared types for right-click menus (see ContextMenu.svelte).

export type MenuItem =
  | {
      label: string;
      action: () => void;
      /** Shown faded on the right, e.g. a keyboard shortcut. */
      hint?: string;
      /** Shown in red, for actions like removing something. */
      danger?: boolean;
      disabled?: boolean;
      /** A coloured dot before the label (a colour name from colors.ts), e.g. a board's colour. */
      color?: string | null;
    }
  | {
      /** A row of colour swatches (plus "no colour"), with `current` marked. */
      current: string | null;
      pick: (color: string | null) => void;
    }
  | "separator";

export interface MenuRequest {
  x: number;
  y: number;
  items: MenuItem[];
}
