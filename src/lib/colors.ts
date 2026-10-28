// The colours workspaces, boards and tags can be given. The library stores only the name
// (`COLORS` in library.rs must list the same names); what each looks like is decided here.

export type ColorName = "red" | "orange" | "yellow" | "green" | "teal" | "blue" | "purple" | "pink";

/** Each colour as it's shown: picked to read well on the dark theme, and distinct from each other. */
export const COLORS: { name: ColorName; label: string; hex: string }[] = [
  { name: "red", label: "Red", hex: "#f06a6a" },
  { name: "orange", label: "Orange", hex: "#f59a4a" },
  { name: "yellow", label: "Yellow", hex: "#e8c94a" },
  { name: "green", label: "Green", hex: "#6cc26c" },
  { name: "teal", label: "Teal", hex: "#45c2b5" },
  { name: "blue", label: "Blue", hex: "#5b9dff" },
  { name: "purple", label: "Purple", hex: "#a57cf0" },
  { name: "pink", label: "Pink", hex: "#ec7ab8" },
];

const byName = new Map(COLORS.map((c) => [c.name, c]));

/** The solid colour (for icons, dots and outlines), or undefined for none or an unknown name. */
export function colorHex(name: string | null | undefined): string | undefined {
  return name ? byName.get(name as ColorName)?.hex : undefined;
}

/** The colour at a given opacity (0–1), for tinted backgrounds. Plain rgba() works in every web view. */
export function colorTint(name: string | null | undefined, opacity: number): string | undefined {
  const hex = colorHex(name);
  if (!hex) return undefined;
  const n = parseInt(hex.slice(1), 16);
  return `rgba(${n >> 16}, ${(n >> 8) & 255}, ${n & 255}, ${opacity})`;
}

export function colorLabel(name: string | null | undefined): string {
  return (name && byName.get(name as ColorName)?.label) || "No color";
}
