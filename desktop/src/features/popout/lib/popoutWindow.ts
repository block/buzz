import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import type { PlaygroundCard } from "@/features/playground/lib/types";

import { hidePlaygroundWebview } from "@/features/playground/lib/webview";
import { dismissPlayground } from "@/features/playground/lib/sessions";

import { openEmbeddedWindow } from "./embeddedWindows";
import {
  isEmbedInMainEnabled,
  isStartFullscreenEnabled,
} from "./popoutSettings";

export const POPOUT_STORAGE_PREFIX = "buzz.popout.v1:";
export const POPOUT_WINDOW_LABEL_PREFIX = "popout-";

export type PopoutKind = "thread" | "playground" | "split";

export type PopoutPayload = {
  kind: PopoutKind;
  title?: string;
  channelId?: string;
  threadId?: string;
  playground?: PlaygroundCard;
};

function sanitizeLabelPart(value: string): string {
  const cleaned = value
    .replace(/[^a-zA-Z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return cleaned.slice(0, 48) || "x";
}

/** Tiny non-crypto hash (cyrb53) so long sid+channel prefixes cannot collide. */
function hashLabelSeed(value: string): string {
  let h1 = 0xdeadbeef;
  let h2 = 0x41c6ce57;
  for (let i = 0; i < value.length; i++) {
    const ch = value.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507);
  h1 ^= Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507);
  h2 ^= Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  const a = (h1 >>> 0).toString(16).padStart(8, "0");
  const b = (h2 >>> 0).toString(16).padStart(8, "0");
  return `${a}${b}`.slice(0, 16);
}

export function popoutStorageKey(label: string): string {
  return `${POPOUT_STORAGE_PREFIX}${label}`;
}

export function popoutLabel(kind: PopoutKind, seed: string): string {
  const hash = sanitizeLabelPart(hashLabelSeed(`${kind}:${seed}`));
  return `${POPOUT_WINDOW_LABEL_PREFIX}${kind}-${hash}`;
}

/** Tauri `invoke` often rejects with a raw string, not an Error. */
export function popoutErrorMessage(error: unknown, fallback: string): string {
  return error instanceof Error
    ? error.message
    : typeof error === "string"
      ? error
      : fallback;
}

export function writePopoutPayload(label: string, payload: PopoutPayload) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      popoutStorageKey(label),
      JSON.stringify(payload),
    );
  } catch {
    // Companion still opens; it just will not know its target.
  }
}

export function readPopoutPayload(
  label = currentPopoutLabel(),
): PopoutPayload | null {
  if (!label || typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(popoutStorageKey(label));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PopoutPayload;
    if (
      parsed?.kind !== "thread" &&
      parsed?.kind !== "playground" &&
      parsed?.kind !== "split"
    ) {
      return null;
    }
    return parsed;
  } catch {
    return null;
  }
}

export function currentPopoutLabel(): string | null {
  if (!isTauri()) return null;
  try {
    const label = getCurrentWindow().label;
    return label.startsWith(POPOUT_WINDOW_LABEL_PREFIX) ? label : null;
  } catch {
    return null;
  }
}

export function currentPopoutPayload(): PopoutPayload | null {
  const label = currentPopoutLabel();
  return label ? readPopoutPayload(label) : null;
}

/** Thread/split pop-outs show only the thread, never the parent channel. */
export function isPopoutThreadOnlyLayout(
  payload: PopoutPayload | null = currentPopoutPayload(),
): boolean {
  if (!payload?.threadId) return false;
  return payload.kind === "thread" || payload.kind === "split";
}

export function popoutPayloadFromInput(input: {
  kind: PopoutKind;
  title: string;
  channelId?: string;
  threadId?: string;
  playground?: PlaygroundCard;
}): PopoutPayload {
  return {
    kind: input.kind,
    title: input.title,
    ...(input.channelId ? { channelId: input.channelId } : {}),
    ...(input.threadId ? { threadId: input.threadId } : {}),
    ...(input.playground ? { playground: input.playground } : {}),
  };
}

/** Args sent to the Tauri `open_popout_window` command for a new OS window. */
export function popoutCreateInvokeArgs(input: {
  label: string;
  title: string;
  fullscreen?: boolean;
}): { label: string; title: string; fullscreen: boolean } {
  return {
    label: input.label,
    title: input.title,
    fullscreen: input.fullscreen ?? isStartFullscreenEnabled(),
  };
}

export async function openPopoutWindow(input: {
  kind: PopoutKind;
  title: string;
  seed: string;
  channelId?: string;
  threadId?: string;
  playground?: PlaygroundCard;
}): Promise<void> {
  const label = popoutLabel(input.kind, input.seed);
  const payload = popoutPayloadFromInput(input);
  if (isEmbedInMainEnabled()) {
    openEmbeddedWindow({ label, payload });
    return;
  }
  writePopoutPayload(label, payload);
  // OS playground/split gets its own window-scoped WKWebView. Park the main
  // overlay and hide main playground-{sid} so it cannot drift over main.
  if (
    input.playground?.sid &&
    (input.kind === "playground" || input.kind === "split")
  ) {
    dismissPlayground();
    void hidePlaygroundWebview(input.playground.sid);
  }
  if (!isTauri()) {
    return;
  }
  await invoke(
    "open_popout_window",
    popoutCreateInvokeArgs({ label, title: input.title }),
  );
}
