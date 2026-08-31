/**
 * Linux WebKitGTK often fires HTML5 `drop` with an empty payload, then
 * (or just before) the Rust GTK listener emits `os-file-drop` with real
 * paths. This bus hands those paths to the composer that last showed the
 * drop overlay, and dedupes the HTML5 + GTK double-delivery.
 */

import { looksLikeFileName } from "./droppedFiles";

type UploadPaths = (paths: string[]) => void;

let activeUpload: UploadPaths | null = null;
let pending: string[] = [];
let lastKey = "";
let lastAt = 0;
let listening = false;

function unique(values: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const value of values) {
    if (seen.has(value)) continue;
    seen.add(value);
    out.push(value);
  }
  return out;
}

function usable(paths: string[]): string[] {
  return unique(paths.filter(looksLikeFileName));
}

function takeIfFresh(paths: string[]): string[] {
  const next = usable(paths);
  if (next.length === 0) return [];
  const key = next.join("\0");
  const now = Date.now();
  if (key === lastKey && now - lastAt < 800) return [];
  lastKey = key;
  lastAt = now;
  return next;
}

/** The composer currently under an OS file drag. */
export function noteOsFileDropTarget(upload: UploadPaths | null): void {
  activeUpload = upload;
}

/** Paths from the GTK `os-file-drop` event. */
export function receiveOsDropPaths(paths: string[]): void {
  const next = usable(paths);
  if (next.length === 0) return;
  if (activeUpload) {
    const fresh = takeIfFresh(next);
    if (fresh.length > 0) activeUpload(fresh);
    pending = [];
    return;
  }
  pending = next;
}

/** Consume paths stashed because HTML5 drop ran before the GTK event. */
export function takePendingOsDropPaths(): string[] {
  const out = pending;
  pending = [];
  return takeIfFresh(out);
}

/** Dedupe HTML5 path recovery against the GTK event. */
export function consumeDropPaths(paths: string[]): string[] {
  return takeIfFresh(paths);
}

/** One window-level listener for all composer instances. */
export function ensureOsDropListener(): void {
  if (listening) return;
  listening = true;
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<{ paths: string[] }>("os-file-drop", (event) => {
        receiveOsDropPaths(event.payload.paths ?? []);
      }),
    )
    .catch(() => {
      listening = false;
    });
}
