// Read-only diagnostic for an empty Bots page. Publishes nothing.
import { nip19 } from "nostr-tools";

import { relayClient } from "@/shared/api/relayClient";
import { relayMembersFromEvent } from "@/shared/api/relayMembers";
import { getIdentity } from "@/shared/api/tauriIdentity";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_COMMUNITY_BOTS } from "@/shared/constants/kinds";

import { COMMUNITY_BOTS_D_TAG, parseCommunityBotsPayload } from "./catalog";

const KIND_NIP43_MEMBERSHIP_LIST = 13534;

export type CatalogDiagnosticRole = "owner" | "admin" | "member" | "none";

export type CatalogDiagnostic = {
  eventCount: number;
  event: null | {
    id: string;
    createdAt: number;
    authorHex: string;
    authorNpub: string;
    botCount: number;
    rawBotEntries: number | null;
  };
  /** null = membership snapshot (kind 13534) not readable. */
  authorRole: CatalogDiagnosticRole | null;
  myPubkey: string;
  myNpub: string;
  myRole: CatalogDiagnosticRole | null;
  membershipReadable: boolean;
  hint: string;
};

function npub(hex: string): string {
  try {
    return nip19.npubEncode(hex);
  } catch {
    return hex;
  }
}

function rawEntryCount(content: string): number | null {
  try {
    const parsed = JSON.parse(content) as { bots?: unknown };
    return Array.isArray(parsed.bots) ? parsed.bots.length : null;
  } catch {
    return null;
  }
}

export const CATALOG_FIX_HINT =
  'Fix (no action taken here): publish the catalog, a kind 30624 event with d-tag buzz:community-bots and content {"version":1,"bots":[{"id","name","pubkey","source":"openclaw"}]}, from a session signed in as the community owner or an admin, for example from Settings → Communities → Bots on the desktop. The relay rejects the event from anyone else.';

export function buildCatalogDiagnostic(input: {
  events: ReadonlyArray<RelayEvent>;
  membershipEvent: RelayEvent | null;
  myPubkey: string;
}): CatalogDiagnostic {
  const mine = input.myPubkey.toLowerCase();
  const catalogEvents = input.events.filter(
    (e) =>
      e.kind === KIND_COMMUNITY_BOTS &&
      e.tags.some((t) => t[0] === "d" && t[1] === COMMUNITY_BOTS_D_TAG),
  );
  let latest: RelayEvent | null = null;
  for (const e of catalogEvents) {
    if (!latest || e.created_at > latest.created_at) latest = e;
  }
  const members = input.membershipEvent
    ? relayMembersFromEvent(input.membershipEvent)
    : null;
  const roleOf = (hex: string): CatalogDiagnosticRole | null => {
    if (!members) return null;
    return members.find((m) => m.pubkey === hex.toLowerCase())?.role ?? "none";
  };
  const authorHex = latest?.pubkey.toLowerCase() ?? "";
  return {
    eventCount: catalogEvents.length,
    event: latest
      ? {
          id: latest.id,
          createdAt: latest.created_at,
          authorHex,
          authorNpub: npub(authorHex),
          botCount: parseCommunityBotsPayload(latest.content).length,
          rawBotEntries: rawEntryCount(latest.content),
        }
      : null,
    authorRole: latest ? roleOf(authorHex) : null,
    myPubkey: mine,
    myNpub: npub(mine),
    myRole: roleOf(mine),
    membershipReadable: members !== null,
    hint: CATALOG_FIX_HINT,
  };
}

export async function runCatalogDiagnostic(): Promise<CatalogDiagnostic> {
  const [{ pubkey }, events, membershipEvent] = await Promise.all([
    getIdentity(),
    relayClient.fetchEvents({
      kinds: [KIND_COMMUNITY_BOTS],
      "#d": [COMMUNITY_BOTS_D_TAG],
      limit: 50,
    }),
    relayClient
      .fetchFirstEvent({ kinds: [KIND_NIP43_MEMBERSHIP_LIST], limit: 1 })
      .catch(() => null),
  ]);
  return buildCatalogDiagnostic({
    events,
    membershipEvent,
    myPubkey: pubkey,
  });
}

export function isAdminRole(role: CatalogDiagnosticRole | null): boolean {
  return role === "owner" || role === "admin";
}
