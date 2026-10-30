import { localExtraSlotIdsKey } from "@/features/channels/readState/readStateFormat";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

/**
 * localStorage-persisted identity for the read-state manager: the stable
 * client id, this client's slot id, and any extra slot ids allocated when the
 * blob outgrows the single-slot budget (NIP-RS multi-slot mode).
 */

const CLIENT_ID_KEY_PREFIX = "buzz.nip-rs.client-id";
const SLOT_ID_KEY_PREFIX = "buzz.nip-rs.slot-id";

export function generateHex(bytes: number): string {
  const arr = new Uint8Array(bytes);
  crypto.getRandomValues(arr);
  return Array.from(arr, (b) => b.toString(16).padStart(2, "0")).join("");
}

export function getOrCreatePersisted(
  key: string,
  generator: () => string,
): string {
  let value = localStorage.getItem(key);
  if (!value) {
    value = generator();
    setLocalStorageItemWithRecovery(key, value);
  }
  return value;
}

export function clientIdKey(pubkey: string): string {
  return `${CLIENT_ID_KEY_PREFIX}:${pubkey}`;
}

export function slotIdKey(pubkey: string): string {
  return `${SLOT_ID_KEY_PREFIX}:${pubkey}`;
}

export function loadExtraSlotIds(pubkey: string): string[] {
  try {
    const raw = localStorage.getItem(localExtraSlotIdsKey(pubkey));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (v): v is string => typeof v === "string" && v.length > 0,
    );
  } catch {
    return [];
  }
}

export function saveExtraSlotIds(pubkey: string, ids: string[]): void {
  setLocalStorageItemWithRecovery(
    localExtraSlotIdsKey(pubkey),
    JSON.stringify(ids),
  );
}
