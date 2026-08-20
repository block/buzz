import type { Fibre } from "@/features/triage/api";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

export type FibreDotState = "unseen" | "updated";
export type FibreSeenMap = Record<string, number>;

export function fibreSeenStorageKey(
  relayUrl: string | undefined,
  pubkey: string | undefined,
): string {
  return `buzz-fibre-seen.v1:${relayUrl ?? "local"}:${pubkey ?? "anonymous"}`;
}

export function fibreDotState(
  fibre: Pick<Fibre, "updatedAt">,
  seenUpdatedAt: number | undefined,
): FibreDotState | null {
  if (seenUpdatedAt == null) return "unseen";
  if (fibre.updatedAt > seenUpdatedAt) return "updated";
  return null;
}

export function readFibreSeenMap(key: string): FibreSeenMap {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {};
    }
    const map: FibreSeenMap = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (typeof value === "number" && Number.isFinite(value)) {
        map[id] = value;
      }
    }
    return map;
  } catch {
    return {};
  }
}

export function writeFibreSeenMap(key: string, map: FibreSeenMap): void {
  if (typeof window === "undefined") return;
  setLocalStorageItemWithRecovery(key, JSON.stringify(map));
}
