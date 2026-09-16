import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import {
  POPOUT_WINDOW_LABEL_PREFIX,
  type PopoutKind,
  readPopoutPayload,
} from "./popoutWindow";

export const POPOUT_WINDOWS_CHANGED = "popout-windows-changed";

export type PopoutWindowRow = {
  label: string;
  title: string;
};

let windows: PopoutWindowRow[] = [];
let cachedSnapshot: PopoutWindowRow[] = [];
const listeners = new Set<() => void>();
let startPromise: Promise<void> | null = null;

function emit() {
  cachedSnapshot = [...windows];
  for (const listener of listeners) listener();
}

export function filterPopoutWindows(
  rows: Array<{ label: string; title?: string }>,
): Array<{ label: string; title: string }> {
  return rows
    .filter((row) => row.label.startsWith(POPOUT_WINDOW_LABEL_PREFIX))
    .map((row) => ({ label: row.label, title: row.title ?? "" }));
}

export function popoutKindFromLabel(label: string): PopoutKind | null {
  if (!label.startsWith(POPOUT_WINDOW_LABEL_PREFIX)) return null;
  const rest = label.slice(POPOUT_WINDOW_LABEL_PREFIX.length);
  if (rest.startsWith("thread-")) return "thread";
  if (rest.startsWith("playground-")) return "playground";
  if (rest.startsWith("split-")) return "split";
  return null;
}

/** Sids currently shown in an OS pop-out playground or split window. */
export function playgroundSidsHostedInOsPopouts(
  rows: Array<{ label: string }>,
): Set<string> {
  const sids = new Set<string>();
  for (const row of rows) {
    const kind = popoutKindFromLabel(row.label);
    if (kind !== "playground" && kind !== "split") continue;
    const sid = readPopoutPayload(row.label)?.playground?.sid;
    if (sid) sids.add(sid);
  }
  return sids;
}

export function resolvePopoutTitle(row: {
  label: string;
  title?: string;
}): string {
  const native = row.title?.trim();
  if (native) return native;
  const fromPayload = readPopoutPayload(row.label)?.title?.trim();
  if (fromPayload) return fromPayload;
  const kind = popoutKindFromLabel(row.label);
  if (kind === "thread") return "Thread";
  if (kind === "split") return "Split";
  if (kind === "playground") return "Playground";
  return "Window";
}

function applyRows(rows: Array<{ label: string; title?: string }>) {
  windows = filterPopoutWindows(rows).map((row) => ({
    label: row.label,
    title: resolvePopoutTitle(row),
  }));
  emit();
}

async function refresh() {
  if (!isTauri()) return;
  try {
    const rows = await invoke<Array<{ label: string; title?: string }>>(
      "list_popout_windows",
    );
    applyRows(Array.isArray(rows) ? rows : []);
  } catch {
    // Keep the last good list rather than flashing empty on a transient IPC miss.
  }
}

function ensureStarted() {
  if (startPromise) return;
  startPromise = (async () => {
    await refresh();
    if (!isTauri()) return;
    try {
      await listen(POPOUT_WINDOWS_CHANGED, () => {
        void refresh();
      });
    } catch {
      // Tests and non-Tauri hosts have no event plugin.
    }
  })();
}

export function subscribePopoutWindows(listener: () => void): () => void {
  listeners.add(listener);
  ensureStarted();
  return () => {
    listeners.delete(listener);
  };
}

export function getPopoutWindows(): PopoutWindowRow[] {
  return cachedSnapshot;
}

export async function focusPopoutWindow(label: string): Promise<void> {
  if (!isTauri()) return;
  if (!label.startsWith(POPOUT_WINDOW_LABEL_PREFIX)) return;
  await invoke("focus_popout_window", { label });
}

export function setPopoutWindowsForTests(
  rows: Array<{ label: string; title?: string }>,
) {
  applyRows(rows);
}

export function resetPopoutWindowsForTests() {
  windows = [];
  emit();
}
