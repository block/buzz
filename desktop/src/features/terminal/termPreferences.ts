import * as React from "react";

import type { CellMetrics } from "./terminalRenderer.ts";

/** localStorage keys for Settings → Buzz Term. */
export const TERM_PREF_KEYS = {
  cwd: "buzz.term.sessionCwd",
  shell: "buzz.term.shell",
  scrollback: "buzz.term.scrollback",
  fontSize: "buzz.term.fontSize",
  openMode: "buzz.term.openMode",
  sessionHost: "buzz.term.sessionHost",
  sessionName: "buzz.term.sessionName",
} as const;

/** Blank = login-shell / portable-pty default (usually home). */
export const DEFAULT_TERM_CWD = "";

/** Blank = system $SHELL resolution. */
export const DEFAULT_TERM_SHELL = "";

/** Matches buzz-terminal `Size::default().scrollback`. */
export const DEFAULT_TERM_SCROLLBACK = 10_000;
export const MIN_TERM_SCROLLBACK = 100;
export const MAX_TERM_SCROLLBACK = 100_000;

/** Term canvas font size in px. Baseline metrics are designed for 14. */
export type TermFontSize = 12 | 14 | 16 | 18;
export const DEFAULT_TERM_FONT_SIZE: TermFontSize = 12;
export const TERM_FONT_SIZES: readonly TermFontSize[] = [12, 14, 16, 18];

/** Channel header / ⌘J open mode. Left-nav stays maximized + all scope. */
export type TermOpenMode = "docked" | "maximized";
export const DEFAULT_TERM_OPEN_MODE: TermOpenMode = "maximized";

/** Where new Term sessions / handoff Open should land. */
export type TermSessionHost = "buzz-term" | "herdr";
/** Prefer herdr when installed; fallback to in-app Term. */
export const DEFAULT_TERM_SESSION_HOST: TermSessionHost = "buzz-term";

/**
 * Named herdr session for attach/create/tab/handoff when Session host is herdr.
 * Default `buzz`. Blank opts into herdr's unnamed default session.
 */
export const DEFAULT_TERM_SESSION_NAME = "buzz";

const BASE_CELL = {
  width: 8.4,
  height: 17,
  baseline: 13,
  fontPx: 14,
} as const;

type Snapshot = {
  cwd: string;
  shell: string;
  scrollback: number;
  fontSize: TermFontSize;
  openMode: TermOpenMode;
  sessionHost: TermSessionHost;
  sessionName: string;
};

const listeners = new Set<() => void>();
let snapshot: Snapshot = readAll();

function notify() {
  for (const listener of listeners) listener();
}

function readRaw(key: string): string | null {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

function writeRaw(key: string, value: string): void {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // Persistence is best-effort.
  }
}

export function parseTermCwd(value: string | null | undefined): string {
  if (value == null) return DEFAULT_TERM_CWD;
  return value.trim();
}

export function parseTermShell(value: string | null | undefined): string {
  if (value == null) return DEFAULT_TERM_SHELL;
  return value.trim();
}

export function parseTermScrollback(value: string | null | undefined): number {
  if (value == null || value.trim() === "") return DEFAULT_TERM_SCROLLBACK;
  const n = Number.parseInt(value, 10);
  if (!Number.isFinite(n)) return DEFAULT_TERM_SCROLLBACK;
  return Math.min(MAX_TERM_SCROLLBACK, Math.max(MIN_TERM_SCROLLBACK, n));
}

export function parseTermFontSize(
  value: string | null | undefined,
): TermFontSize {
  const n = Number.parseInt(value ?? "", 10);
  return (TERM_FONT_SIZES as readonly number[]).includes(n)
    ? (n as TermFontSize)
    : DEFAULT_TERM_FONT_SIZE;
}

export function parseTermOpenMode(
  value: string | null | undefined,
): TermOpenMode {
  return value === "maximized" || value === "docked"
    ? value
    : DEFAULT_TERM_OPEN_MODE;
}

export function parseTermSessionHost(
  value: string | null | undefined,
): TermSessionHost {
  return value === "buzz-term" || value === "herdr"
    ? value
    : DEFAULT_TERM_SESSION_HOST;
}

/** null/undefined → buzz; stored blank stays blank (herdr default session). */
export function parseTermSessionName(value: string | null | undefined): string {
  if (value == null) return DEFAULT_TERM_SESSION_NAME;
  return value.trim();
}

function readAll(): Snapshot {
  return {
    cwd: parseTermCwd(readRaw(TERM_PREF_KEYS.cwd)),
    shell: parseTermShell(readRaw(TERM_PREF_KEYS.shell)),
    scrollback: parseTermScrollback(readRaw(TERM_PREF_KEYS.scrollback)),
    fontSize: parseTermFontSize(readRaw(TERM_PREF_KEYS.fontSize)),
    openMode: parseTermOpenMode(readRaw(TERM_PREF_KEYS.openMode)),
    sessionHost: parseTermSessionHost(readRaw(TERM_PREF_KEYS.sessionHost)),
    sessionName: parseTermSessionName(readRaw(TERM_PREF_KEYS.sessionName)),
  };
}

function publish(next: Snapshot) {
  snapshot = next;
  notify();
}

export function getTermPreferences(): Snapshot {
  return snapshot;
}

export function setTermCwd(next: string): void {
  const cwd = parseTermCwd(next);
  writeRaw(TERM_PREF_KEYS.cwd, cwd);
  publish({ ...snapshot, cwd });
}

export function setTermShell(next: string): void {
  const shell = parseTermShell(next);
  writeRaw(TERM_PREF_KEYS.shell, shell);
  publish({ ...snapshot, shell });
}

export function setTermScrollback(next: number | string): void {
  const scrollback = parseTermScrollback(String(next));
  writeRaw(TERM_PREF_KEYS.scrollback, String(scrollback));
  publish({ ...snapshot, scrollback });
}

export function setTermFontSize(next: TermFontSize): void {
  writeRaw(TERM_PREF_KEYS.fontSize, String(next));
  publish({ ...snapshot, fontSize: next });
}

export function setTermOpenMode(next: TermOpenMode): void {
  writeRaw(TERM_PREF_KEYS.openMode, next);
  publish({ ...snapshot, openMode: next });
}

export function setTermSessionHost(next: TermSessionHost): void {
  writeRaw(TERM_PREF_KEYS.sessionHost, next);
  publish({ ...snapshot, sessionHost: next });
}

export function setTermSessionName(next: string): void {
  // Blank is intentional (herdr unnamed default). Unset storage still reads as buzz.
  const sessionName = String(next ?? "").trim();
  writeRaw(TERM_PREF_KEYS.sessionName, sessionName);
  publish({ ...snapshot, sessionName });
}

/** Non-React subscription (remote access re-pushes the shell/herdr choice to the Rust backend when it changes). */
export function subscribeTermPreferences(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

export function useTermPreferences(): Snapshot {
  return React.useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getTermPreferences,
    () => ({
      cwd: DEFAULT_TERM_CWD,
      shell: DEFAULT_TERM_SHELL,
      scrollback: DEFAULT_TERM_SCROLLBACK,
      fontSize: DEFAULT_TERM_FONT_SIZE,
      openMode: DEFAULT_TERM_OPEN_MODE,
      sessionHost: DEFAULT_TERM_SESSION_HOST,
      sessionName: DEFAULT_TERM_SESSION_NAME,
    }),
  );
}

/**
 * Resolve cwd for `terminal_attach`.
 * Blank → unset (login-shell / portable-pty home default).
 * `handoffCwd` wins when non-empty (per-tab card override).
 */
export async function resolveTermCwdForAttach(options: {
  preference: string;
  handoffCwd?: string | null;
  homeDir: () => Promise<string>;
}): Promise<string | undefined> {
  const handoff = options.handoffCwd?.trim();
  const raw = handoff || options.preference.trim();
  if (!raw || raw === "~") return undefined;
  if (raw.startsWith("~/")) {
    const home = await options.homeDir();
    const base = home.endsWith("/") ? home.slice(0, -1) : home;
    return `${base}/${raw.slice(2)}`;
  }
  return raw;
}

/** Scale the 14px baseline metrics to the user's Term font size. */
export function terminalCellMetricsForFontSize(
  fontSize: TermFontSize,
): CellMetrics {
  const scale = fontSize / BASE_CELL.fontPx;
  const width = Math.round(BASE_CELL.width * scale * 100) / 100;
  const height = Math.round(BASE_CELL.height * scale);
  const baseline = Math.round(BASE_CELL.baseline * scale);
  return {
    width,
    height,
    baseline,
    font: `${fontSize}px "JetBrains Mono", monospace`,
    boldFont: `700 ${fontSize}px "JetBrains Mono", monospace`,
  };
}

/** @deprecated Use termPreferences — kept for any stray imports during transition. */
export const TERM_SESSION_CWD_STORAGE_KEY = TERM_PREF_KEYS.cwd;
export const DEFAULT_TERM_SESSION_CWD = DEFAULT_TERM_CWD;
export const getTermSessionCwd = () => getTermPreferences().cwd;
export const setTermSessionCwd = setTermCwd;
export const useTermSessionCwd = () => useTermPreferences().cwd;
export const parseTermSessionCwd = parseTermCwd;
export async function resolveTermSessionCwdForAttach(
  preference: string,
  homeDir: () => Promise<string>,
): Promise<string | undefined> {
  return resolveTermCwdForAttach({ preference, homeDir });
}

export function resetTermPreferencesForTests(): void {
  snapshot = {
    cwd: DEFAULT_TERM_CWD,
    shell: DEFAULT_TERM_SHELL,
    scrollback: DEFAULT_TERM_SCROLLBACK,
    fontSize: DEFAULT_TERM_FONT_SIZE,
    openMode: DEFAULT_TERM_OPEN_MODE,
    sessionHost: DEFAULT_TERM_SESSION_HOST,
    sessionName: DEFAULT_TERM_SESSION_NAME,
  };
  for (const key of Object.values(TERM_PREF_KEYS)) {
    try {
      globalThis.localStorage?.removeItem(key);
    } catch {
      // ignore
    }
  }
  notify();
}
