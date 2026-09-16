import * as React from "react";

import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

export const SHOW_WINDOWS_SECTION_KEY = "hula.popout.showWindowsSection";
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

let showWindowsSection = readBool(SHOW_WINDOWS_SECTION_KEY, true);
let startFullscreen = readBool(START_FULLSCREEN_KEY, false);
let embedInMain = readBool(EMBED_IN_MAIN_KEY, false);

// Fullscreen (OS windows) and embed (in-app) cannot both be on. If both were
// stored true, drop embed so leftover OS windows keep their fullscreen pref.
if (startFullscreen && embedInMain) {
  embedInMain = false;
  writeBool(EMBED_IN_MAIN_KEY, false);
}

export type PopoutSettings = {
  showWindowsSection: boolean;
  startFullscreen: boolean;
  embedInMain: boolean;
};

let cachedSnapshot: PopoutSettings = {
  showWindowsSection,
  startFullscreen,
  embedInMain,
};

function emit() {
  cachedSnapshot = {
    showWindowsSection,
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

export function isShowWindowsSectionEnabled(): boolean {
  return showWindowsSection;
}

export function isStartFullscreenEnabled(): boolean {
  return startFullscreen;
}

/** Embed is only effective while the Windows section is visible. */
export function isEmbedInMainEnabled(): boolean {
  return showWindowsSection && embedInMain;
}

export function setShowWindowsSection(enabled: boolean): void {
  showWindowsSection = enabled;
  writeBool(SHOW_WINDOWS_SECTION_KEY, enabled);
  if (!enabled && embedInMain) {
    embedInMain = false;
    writeBool(EMBED_IN_MAIN_KEY, false);
  }
  emit();
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
  embedInMain = enabled && showWindowsSection;
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

export function useShowWindowsSection(): boolean {
  return usePopoutSettings().showWindowsSection;
}

export function useStartFullscreen(): boolean {
  return usePopoutSettings().startFullscreen;
}

export function useEmbedInMainEnabled(): boolean {
  const settings = usePopoutSettings();
  return settings.showWindowsSection && settings.embedInMain;
}

export function resetPopoutSettingsForTests(): void {
  showWindowsSection = true;
  startFullscreen = false;
  embedInMain = false;
  emit();
}
