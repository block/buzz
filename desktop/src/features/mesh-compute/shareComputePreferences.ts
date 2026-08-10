import * as React from "react";

/** Device-local Share Compute choices, shared by Settings and the sidebar card. */
export const SHARE_COMPUTE_MODEL_STORAGE_KEY =
  "buzz.mesh-compute.share.model.v1";
export const SHARE_COMPUTE_MAX_VRAM_STORAGE_KEY =
  "buzz.mesh-compute.share.max-vram-gb.v1";

const listeners = new Set<() => void>();
let model = readPreference(SHARE_COMPUTE_MODEL_STORAGE_KEY);
let maxVramGb = readPreference(SHARE_COMPUTE_MAX_VRAM_STORAGE_KEY);

function readPreference(key: string): string {
  try {
    return globalThis.localStorage?.getItem(key) ?? "";
  } catch {
    return "";
  }
}

function writePreference(key: string, value: string): void {
  try {
    if (value === "") {
      globalThis.localStorage?.removeItem(key);
    } else {
      globalThis.localStorage?.setItem(key, value);
    }
  } catch {
    // Persistence is best-effort; the in-memory choice still applies.
  }
}

function emitChange(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getShareComputeModel(): string {
  return model;
}

export function setShareComputeModel(value: string): void {
  if (model === value) return;
  model = value;
  writePreference(SHARE_COMPUTE_MODEL_STORAGE_KEY, value);
  emitChange();
}

export function useShareComputeModel(): string {
  return React.useSyncExternalStore(subscribe, getShareComputeModel, () => "");
}

export function getShareComputeMaxVramGb(): string {
  return maxVramGb;
}

export function setShareComputeMaxVramGb(value: string): void {
  if (maxVramGb === value) return;
  maxVramGb = value;
  writePreference(SHARE_COMPUTE_MAX_VRAM_STORAGE_KEY, value);
  emitChange();
}

export function useShareComputeMaxVramGb(): string {
  return React.useSyncExternalStore(
    subscribe,
    getShareComputeMaxVramGb,
    () => "",
  );
}

/** Settings choice wins; recommendation is only the first-run fallback. */
export function resolveShareComputeModel(
  selected: string,
  recommended: string | null | undefined,
): string | null {
  return selected.trim() || recommended || null;
}
