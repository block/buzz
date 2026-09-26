import * as React from "react";

import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

export const START_FULLSCREEN_KEY = "hula.popout.startFullscreen";
export const EMBED_IN_MAIN_KEY = "hula.popout.embedInMain";

const listeners = new Set<() => void>();

function readBool(key: string, fallback: boolean): boolean {
  if (typeof window === "undefined") return fallback;
  const raw = getStorageItem(key);
  if (raw == null) return fallback;
  return raw === "true";
}

function writeBool(key: string, value: boolean): void {
  setStorageItem(key, value ? "true" : "false");
}

let startFullscreen = readBool(START_FULLSCREEN_KEY, false);
let embedInMain = readBool(EMBED_IN_MAIN_KEY, false);

// Fullscreen (OS windows) and embed (in-app) cannot both be on. If both were
// stored true, drop embed so leftover OS windows keep their fullscreen pref.
if (startFullscreen && embedInMain) {
  embedInMain = false;
  writeBool(EMBED_IN_MAIN_KEY, false);
}

export type PopoutSettings = {
  startFullscreen: boolean;
  embedInMain: boolean;
};

let cachedSnapshot: PopoutSettings = {
  startFullscreen,
  embedInMain,
};

function emit() {
  cachedSnapshot = {
    startFullscreen,
    embedInMain,
  };
  for (const listener of listeners) listener();
}

export function subscribePopoutSettings(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPopoutSettings(): PopoutSettings {
  return cachedSnapshot;
}

export function isStartFullscreenEnabled(): boolean {
  return startFullscreen;
}

export function isEmbedInMainEnabled(): boolean {
  return embedInMain;
}

export function setStartFullscreen(enabled: boolean): void {
  startFullscreen = enabled;
  writeBool(START_FULLSCREEN_KEY, enabled);
  if (enabled && embedInMain) {
    embedInMain = false;
    writeBool(EMBED_IN_MAIN_KEY, false);
  }
  emit();
}

export function setEmbedInMain(enabled: boolean): void {
  embedInMain = enabled;
  writeBool(EMBED_IN_MAIN_KEY, embedInMain);
  if (embedInMain && startFullscreen) {
    startFullscreen = false;
    writeBool(START_FULLSCREEN_KEY, false);
  }
  emit();
}

export function usePopoutSettings(): PopoutSettings {
  return React.useSyncExternalStore(
    subscribePopoutSettings,
    getPopoutSettings,
    getPopoutSettings,
  );
}

export function useStartFullscreen(): boolean {
  return usePopoutSettings().startFullscreen;
}

export function useEmbedInMainEnabled(): boolean {
  return usePopoutSettings().embedInMain;
}

export function resetPopoutSettingsForTests(): void {
  startFullscreen = false;
  embedInMain = false;
  emit();
}
