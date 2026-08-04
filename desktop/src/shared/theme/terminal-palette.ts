import type { ThemeRegistrationRaw } from "shiki";

export const ANSI_COLOR_NAMES = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

export type AnsiColorName = (typeof ANSI_COLOR_NAMES)[number];

export type TerminalPalette = {
  background: string;
  foreground: string;
  cursor: string;
  cursorText: string;
  ansi: Readonly<Record<AnsiColorName, string>>;
};

type TokenSetting = {
  scope?: string | readonly string[];
  settings?: { foreground?: string };
};

type Rgb = readonly [number, number, number];

const FALLBACK_HUES: Readonly<Record<string, string>> = {
  red: "#d75f5f",
  green: "#5faf87",
  yellow: "#d7af5f",
  blue: "#5f87d7",
  magenta: "#af87d7",
  cyan: "#5fafd7",
};

const SCOPE_ANCHORS: Readonly<Record<string, readonly string[]>> = {
  red: ["invalid", "keyword", "deleted"],
  green: ["string", "inserted", "tag"],
  yellow: ["type", "class", "escape"],
  blue: ["function", "call", "method"],
  magenta: ["numeric", "constant", "operator"],
  cyan: ["attribute", "parameter", "property"],
};

function parseHex(value: string | undefined): Rgb | null {
  if (!value) return null;
  const match = /^#([\da-f]{3}|[\da-f]{6}|[\da-f]{8})$/i.exec(value.trim());
  if (!match) return null;
  let hex = match[1];
  if (hex.length === 3) hex = [...hex].map((part) => `${part}${part}`).join("");
  if (hex.length === 8) hex = hex.slice(0, 6);
  return [
    Number.parseInt(hex.slice(0, 2), 16),
    Number.parseInt(hex.slice(2, 4), 16),
    Number.parseInt(hex.slice(4, 6), 16),
  ];
}

function toHex(rgb: Rgb): string {
  return `#${rgb
    .map((channel) => Math.round(channel).toString(16).padStart(2, "0"))
    .join("")}`;
}

function normalizeHex(value: string | undefined, fallback: string): string {
  const color = parseHex(value);
  return color ? toHex(color) : fallback;
}

function mix(from: string, to: string, amount: number): string {
  const a = parseHex(from);
  const b = parseHex(to);
  if (!a || !b) return from;
  return toHex([
    a[0] + (b[0] - a[0]) * amount,
    a[1] + (b[1] - a[1]) * amount,
    a[2] + (b[2] - a[2]) * amount,
  ]);
}

function luminance(color: string): number {
  const rgb = parseHex(color) ?? [0, 0, 0];
  const channels = rgb.map((channel) => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
}

function tokenSettings(theme: ThemeRegistrationRaw): readonly TokenSetting[] {
  const source =
    (theme as ThemeRegistrationRaw & { tokenColors?: readonly TokenSetting[] })
      .tokenColors ?? (theme.settings as readonly TokenSetting[] | undefined);
  return source ?? [];
}

function scopeColor(
  settings: readonly TokenSetting[],
  anchors: readonly string[],
): string | undefined {
  for (const entry of settings) {
    if (!entry.settings?.foreground || !entry.scope) continue;
    const scopes = Array.isArray(entry.scope) ? entry.scope : [entry.scope];
    if (
      scopes.some((scope) =>
        anchors.some((anchor) =>
          scope
            .toLowerCase()
            .split(/[.,\s]+/)
            .some((part: string) => part.includes(anchor)),
        ),
      )
    ) {
      return entry.settings.foreground;
    }
  }
  return undefined;
}

/** Derive a complete terminal palette from the same immutable Shiki theme. */
export function extractTerminalPalette(
  theme: ThemeRegistrationRaw,
): TerminalPalette {
  const colors = (theme.colors ?? {}) as Readonly<Record<string, string>>;
  const background = normalizeHex(colors["editor.background"], "#1e1e1e");
  const foreground = normalizeHex(colors["editor.foreground"], "#d4d4d4");
  const isLight = luminance(background) > luminance(foreground);
  const brightenToward = isLight ? "#000000" : "#ffffff";
  const settings = tokenSettings(theme);

  const base: Record<string, string> = {
    black: normalizeHex(
      colors["terminal.ansiBlack"],
      mix(background, foreground, 0.16),
    ),
    white: normalizeHex(
      colors["terminal.ansiWhite"],
      mix(background, foreground, 0.82),
    ),
  };

  for (const name of ["red", "green", "yellow", "blue", "magenta", "cyan"]) {
    const key = `terminal.ansi${name[0].toUpperCase()}${name.slice(1)}`;
    const anchored = scopeColor(settings, SCOPE_ANCHORS[name]);
    const fallback = mix(FALLBACK_HUES[name], foreground, 0.18);
    base[name] = normalizeHex(colors[key], normalizeHex(anchored, fallback));
  }

  const ansi = {} as Record<AnsiColorName, string>;
  for (const name of ANSI_COLOR_NAMES.slice(0, 8)) {
    ansi[name] = base[name];
  }
  ANSI_COLOR_NAMES.slice(8).forEach((brightName, index) => {
    const baseName = ANSI_COLOR_NAMES[index];
    const key = `terminal.ansi${brightName[0].toUpperCase()}${brightName.slice(1)}`;
    ansi[brightName] = normalizeHex(
      colors[key],
      mix(base[baseName], brightenToward, 0.28),
    );
  });

  return {
    background,
    foreground,
    cursor: normalizeHex(colors["terminalCursor.foreground"], foreground),
    cursorText: normalizeHex(colors["terminalCursor.background"], background),
    ansi,
  };
}
