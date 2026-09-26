import { normalizeRelayUrl } from "@/shared/lib/normalizeRelayUrl";

import type { SiteRunbookRef } from "./types";

const STORAGE_KEY_PREFIX = "buzz-site-runbooks.v1";

export function siteRunbooksStorageKey(
  pubkey: string,
  relayUrl: string,
): string {
  return `${STORAGE_KEY_PREFIX}:${pubkey}:${encodeURIComponent(normalizeRelayUrl(relayUrl))}`;
}

export function encodeRunbookRef(ref: SiteRunbookRef): string {
  if (ref.kind === "pin") {
    return `pin:${ref.pinId.trim()}`;
  }
  return `sid:${ref.sid.trim()}`;
}

export function parseRunbookRef(raw: string): SiteRunbookRef | null {
  const value = raw.trim();
  if (value.startsWith("pin:")) {
    const pinId = value.slice(4).trim();
    if (!pinId) return null;
    return { kind: "pin", pinId };
  }
  if (value.startsWith("sid:")) {
    const sid = value.slice(4).trim();
    if (!sid) return null;
    return { kind: "sid", sid };
  }
  return null;
}

export function pinRunbookRef(pinId: string): SiteRunbookRef {
  return { kind: "pin", pinId: pinId.trim() };
}

export function sidRunbookRef(sid: string): SiteRunbookRef {
  return { kind: "sid", sid: sid.trim() };
}
