import * as React from "react";

/** Device-level chrome contrast applied across the whole interface. */
export type InterfaceContrast = "low" | "default" | "high";

export const INTERFACE_CONTRAST_STORAGE_KEY =
  "buzz.appearance.interfaceContrast";
export const DEFAULT_INTERFACE_CONTRAST: InterfaceContrast = "default";

const listeners = new Set<() => void>();
let interfaceContrast: InterfaceContrast = DEFAULT_INTERFACE_CONTRAST;
let listeningForStorageChanges = false;

export function parseInterfaceContrast(
  value: string | null | undefined,
): InterfaceContrast {
  return value === "low" || value === "default" || value === "high"
    ? value
    : DEFAULT_INTERFACE_CONTRAST;
}

function readStoredInterfaceContrast(): InterfaceContrast {
  try {
    return parseInterfaceContrast(
      globalThis.localStorage?.getItem(INTERFACE_CONTRAST_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_INTERFACE_CONTRAST;
  }
}

function applyInterfaceContrast(contrast: InterfaceContrast): void {
  globalThis.document?.documentElement?.setAttribute(
    "data-interface-contrast",
    contrast,
  );
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

function applyStoredInterfaceContrast(): void {
  const nextContrast = readStoredInterfaceContrast();
  const changed = nextContrast !== interfaceContrast;
  interfaceContrast = nextContrast;
  applyInterfaceContrast(nextContrast);
  if (changed) notifyListeners();
}

function listenForStorageChanges(): void {
  if (listeningForStorageChanges || !globalThis.window?.addEventListener)
    return;
  globalThis.window.addEventListener("storage", (event) => {
    if (event.key === INTERFACE_CONTRAST_STORAGE_KEY || event.key === null) {
      applyStoredInterfaceContrast();
    }
  });
  listeningForStorageChanges = true;
}

/** Apply the persisted preference before React renders to avoid a flash of chrome. */
export function initializeInterfaceContrastPreference(): void {
  applyStoredInterfaceContrast();
  listenForStorageChanges();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getInterfaceContrast(): InterfaceContrast {
  return interfaceContrast;
}

export function setInterfaceContrast(contrast: InterfaceContrast): void {
  interfaceContrast = contrast;
  applyInterfaceContrast(contrast);
  try {
    globalThis.localStorage?.setItem(INTERFACE_CONTRAST_STORAGE_KEY, contrast);
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  notifyListeners();
}

/** Temporarily apply a contrast without changing the saved preference. */
export function previewInterfaceContrast(
  contrast: InterfaceContrast | null,
): void {
  applyInterfaceContrast(contrast ?? interfaceContrast);
}

export function useInterfaceContrast(): InterfaceContrast {
  return React.useSyncExternalStore(
    subscribe,
    getInterfaceContrast,
    () => DEFAULT_INTERFACE_CONTRAST,
  );
}
