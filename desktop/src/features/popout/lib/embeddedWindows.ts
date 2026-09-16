import { hideAllPinWebviews } from "@/features/pinned-sites/lib/pinWebview";
import { hidePlaygroundWebview } from "@/features/playground/lib/webview";
import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

import type { PlaygroundCard } from "@/features/playground/lib/types";

import { isEmbedInMainEnabled } from "./popoutSettings";

export type EmbeddedPayload = {
  kind: "thread" | "playground" | "split";
  title?: string;
  channelId?: string;
  threadId?: string;
  playground?: PlaygroundCard;
};

export const EMBEDDED_WINDOWS_STORAGE_KEY = "hula.popout.embeddedWindows";

export type EmbeddedWindow = {
  label: string;
  title: string;
  payload: EmbeddedPayload;
};

type EmbeddedStore = {
  windows: EmbeddedWindow[];
  activeLabel: string | null;
};

const store: EmbeddedStore = {
  windows: [],
  activeLabel: null,
};

let cachedSnapshot: EmbeddedStore = {
  windows: [],
  activeLabel: null,
};
const listeners = new Set<() => void>();

function snapshot(): EmbeddedStore {
  return {
    windows: [...store.windows],
    activeLabel: store.activeLabel,
  };
}

function emit() {
  cachedSnapshot = snapshot();
  for (const listener of listeners) listener();
}

function persist() {
  setStorageItem(
    EMBEDDED_WINDOWS_STORAGE_KEY,
    JSON.stringify({
      windows: store.windows,
      activeLabel: store.activeLabel,
    }),
  );
}

function isPayload(value: unknown): value is EmbeddedPayload {
  if (!value || typeof value !== "object") return false;
  const kind = (value as EmbeddedPayload).kind;
  return kind === "thread" || kind === "playground" || kind === "split";
}

function load() {
  if (typeof window === "undefined") return;
  const raw = getStorageItem(EMBEDDED_WINDOWS_STORAGE_KEY);
  if (!raw) return;
  try {
    const saved = JSON.parse(raw) as {
      windows?: EmbeddedWindow[];
      activeLabel?: string | null;
    };
    store.windows = [];
    for (const row of saved.windows ?? []) {
      if (row?.label && row.title != null && isPayload(row.payload)) {
        store.windows.push({
          label: row.label,
          title: row.title,
          payload: row.payload,
        });
      }
    }
    store.activeLabel =
      typeof saved.activeLabel === "string" &&
      store.windows.some((row) => row.label === saved.activeLabel)
        ? saved.activeLabel
        : null;
  } catch {
    store.windows = [];
    store.activeLabel = null;
  }
}

load();
cachedSnapshot = snapshot();

export function subscribeEmbeddedWindows(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getEmbeddedWindowsStore(): EmbeddedStore {
  return cachedSnapshot;
}

export function listEmbeddedWindows(): EmbeddedWindow[] {
  return cachedSnapshot.windows;
}

export function getActiveEmbeddedWindow(): EmbeddedWindow | null {
  if (!isEmbedInMainEnabled()) return null;
  const { windows, activeLabel } = cachedSnapshot;
  if (!activeLabel) return null;
  return windows.find((row) => row.label === activeLabel) ?? null;
}

let onEmbedOpen = () => {};

export function registerEmbedOpenHandler(handler: () => void) {
  onEmbedOpen = handler;
}

function hideOverlays() {
  onEmbedOpen();
  void hideAllPinWebviews();
}

export function openEmbeddedWindow(input: {
  label: string;
  payload: EmbeddedPayload;
}): EmbeddedWindow {
  const title = input.payload.title?.trim() || defaultTitle(input.payload.kind);
  const payload: EmbeddedPayload = { ...input.payload, title };
  const existing = store.windows.find((row) => row.label === input.label);
  if (existing) {
    existing.payload = payload;
    existing.title = title;
    store.activeLabel = existing.label;
    persist();
    emit();
    hideOverlays();
    return existing;
  }
  const row: EmbeddedWindow = {
    label: input.label,
    title,
    payload,
  };
  store.windows.push(row);
  store.activeLabel = row.label;
  persist();
  emit();
  hideOverlays();
  return row;
}

export function showEmbeddedWindow(label: string) {
  const row = store.windows.find((entry) => entry.label === label);
  if (!row) return;
  const previous =
    store.activeLabel != null && store.activeLabel !== label
      ? store.windows.find((entry) => entry.label === store.activeLabel)
      : null;
  const previousSid = previous?.payload.playground?.sid ?? null;
  store.activeLabel = label;
  persist();
  emit();
  hideOverlays();
  // Leaving an embed playground/split must hide its window-scoped webview.
  if (previousSid && previousSid !== row.payload.playground?.sid) {
    void hidePlaygroundWebview(previousSid);
  }
}

export function dismissEmbeddedWindow() {
  if (store.activeLabel == null) return;
  const active = store.windows.find((row) => row.label === store.activeLabel);
  const sid = active?.payload.playground?.sid ?? null;
  store.activeLabel = null;
  persist();
  emit();
  if (sid) void hidePlaygroundWebview(sid);
}

export function closeEmbeddedWindow(label: string) {
  const closing = store.windows.find((row) => row.label === label);
  const sid = closing?.payload.playground?.sid ?? null;
  const wasActive = store.activeLabel === label;
  store.windows = store.windows.filter((row) => row.label !== label);
  if (wasActive) store.activeLabel = null;
  persist();
  emit();
  // Always hide: an inactive row can still own a leaked topmost child if a
  // prior dismiss raced an in-flight show.
  if (sid) void hidePlaygroundWebview(sid);
}

function defaultTitle(kind: EmbeddedPayload["kind"]): string {
  if (kind === "thread") return "Thread";
  if (kind === "split") return "Split";
  return "Playground";
}

export function resetEmbeddedWindowsForTests() {
  store.windows = [];
  store.activeLabel = null;
  emit();
}
