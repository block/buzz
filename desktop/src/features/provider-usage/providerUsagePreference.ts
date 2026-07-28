import { useSyncExternalStore } from "react";

import type {
  ProviderUsageCapability,
  ProviderUsageId,
  ProviderUsagePreference,
} from "@/shared/api/tauriProviderUsage";

const STORAGE_KEY = "buzz-provider-usage-preference";
const CHANGE_EVENT = "buzz-provider-usage-preference-change";

function parsePreference(value: string | null): ProviderUsagePreference {
  if (
    value === "codex" ||
    value === "claude" ||
    value === "grok" ||
    value === "auto"
  ) {
    return value;
  }
  return "auto";
}

export function getProviderUsagePreference(): ProviderUsagePreference {
  if (typeof window === "undefined") return "auto";
  try {
    return parsePreference(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return "auto";
  }
}

export function setProviderUsagePreference(
  preference: ProviderUsagePreference,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Device-level UI preference. Storage failure safely falls back to Auto.
  }
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

function subscribe(onStoreChange: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, onStoreChange);
  window.addEventListener("storage", onStoreChange);
  return () => {
    window.removeEventListener(CHANGE_EVENT, onStoreChange);
    window.removeEventListener("storage", onStoreChange);
  };
}

export function useProviderUsagePreference(): ProviderUsagePreference {
  return useSyncExternalStore(
    subscribe,
    getProviderUsagePreference,
    () => "auto",
  );
}

export function resolveProviderUsagePreference(
  preference: ProviderUsagePreference,
  capabilities: ProviderUsageCapability[] = [],
): ProviderUsageId {
  if (preference !== "auto") return preference;
  return (
    capabilities.find((capability) => capability.availability === "available")
      ?.id ?? "codex"
  );
}
