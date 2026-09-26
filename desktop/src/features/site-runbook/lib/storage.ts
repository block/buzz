import { getStorageItem, setStorageItem } from "@/shared/lib/safeStorage";

import { siteRunbooksStorageKey } from "./keys";
import {
  emptyRunbooksBlob,
  parseSiteRunbooksBlob,
} from "./serialize";
import type { SiteRunbook, SiteRunbooksBlob } from "./types";

export function loadSiteRunbooksBlob(
  pubkey: string,
  relayUrl: string,
): SiteRunbooksBlob {
  const raw = getStorageItem(siteRunbooksStorageKey(pubkey, relayUrl));
  if (!raw) return emptyRunbooksBlob();
  try {
    const parsed = parseSiteRunbooksBlob(JSON.parse(raw));
    return parsed ?? emptyRunbooksBlob();
  } catch {
    return emptyRunbooksBlob();
  }
}

export function saveSiteRunbooksBlob(
  pubkey: string,
  relayUrl: string,
  blob: SiteRunbooksBlob,
): void {
  setStorageItem(
    siteRunbooksStorageKey(pubkey, relayUrl),
    JSON.stringify({
      version: 1,
      runbooks: blob.runbooks,
    }),
  );
}

export function getRunbookFromBlob(
  blob: SiteRunbooksBlob,
  encodedKey: string,
): SiteRunbook | null {
  return blob.runbooks[encodedKey] ?? null;
}

export function upsertRunbookInBlob(
  blob: SiteRunbooksBlob,
  encodedKey: string,
  runbook: SiteRunbook,
): SiteRunbooksBlob {
  return {
    version: 1,
    runbooks: {
      ...blob.runbooks,
      [encodedKey]: runbook,
    },
  };
}

export function removeRunbookFromBlob(
  blob: SiteRunbooksBlob,
  encodedKey: string,
): SiteRunbooksBlob {
  if (!(encodedKey in blob.runbooks)) return blob;
  const runbooks = { ...blob.runbooks };
  delete runbooks[encodedKey];
  return { version: 1, runbooks };
}
