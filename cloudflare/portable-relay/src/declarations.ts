// Journal-derived operator configuration.
//
// Evaluates owner-signed sync declaration heads (kind 30700) into the
// node's operating configuration, per the runtime evaluation policy in
// specs/architecture/sovereign-sync-agreement-v0.1-draft.md: only heads
// authored by the node's owner govern; a domain with any owner-signed head
// is governed wholesale by the journal (env config ignored); only
// `status: "active"` heads confer trust; a malformed active head is a loud
// error, never silently dropped trust.

import type { Event } from "nostr-tools";
import type { ReplicationPeerTrust, StreamExport } from "./replication";

/** PROVISIONAL kind pending upstream registry assignment (buzz-core/src/kind.rs). */
export const KIND_SYNC_DECLARATION = 30700;

/**
 * Per-domain evaluation result. `null` means the owner has published no head
 * in that domain — the domain is unclaimed and env bootstrap config may
 * govern. A non-null (possibly empty) record means the journal governs the
 * domain wholesale; all-revoked is empty trust, not a fallback.
 */
export interface DeclarationConfig {
  peers: Record<string, ReplicationPeerTrust> | null;
  readers: Record<string, ReplicationPeerTrust> | null;
  streams: Record<string, StreamExport> | null;
}

/** An owner-signed head exists but cannot be interpreted; trust is ambiguous. */
export class MalformedDeclarationError extends Error {
  constructor(dTag: string, eventId: string, reason: string) {
    super(`malformed declaration ${dTag} (event ${eventId}): ${reason}`);
    this.name = "MalformedDeclarationError";
  }
}

/**
 * Evaluates effective owner-signed declaration heads into configuration.
 *
 * Only heads `n`-tagged with `nodeLabel` govern: journals replicate whole, so
 * one owner's declarations for different nodes coexist in every copy of the
 * journal, and each node must evaluate only its own.
 */
export function evaluateDeclarations(
  events: Event[],
  owner: string,
  nodeLabel: string,
): DeclarationConfig {
  let peers: Record<string, ReplicationPeerTrust> | null = null;
  let readers: Record<string, ReplicationPeerTrust> | null = null;
  let streams: Record<string, StreamExport> | null = null;

  for (const event of events) {
    if (event.kind !== KIND_SYNC_DECLARATION || event.pubkey !== owner) {
      continue;
    }
    const dTag = event.tags.find((tag) => tag[0] === "d")?.[1];
    if (dTag === undefined) {
      continue;
    }
    const nTag = event.tags.find((tag) => tag[0] === "n")?.[1];
    if (nTag !== nodeLabel) {
      continue;
    }
    if (dTag.startsWith("admit/")) {
      peers ??= {};
      const entry = peerEntry(event, dTag);
      if (entry !== null) {
        peers[dTag.slice("admit/".length)] = entry;
      }
    } else if (dTag.startsWith("read/")) {
      readers ??= {};
      const entry = peerEntry(event, dTag);
      if (entry !== null) {
        readers[dTag.slice("read/".length)] = entry;
      }
    } else if (dTag.startsWith("export/")) {
      streams ??= {};
      const selection = exportEntry(event, dTag);
      if (selection !== null) {
        streams[dTag.slice("export/".length)] = selection;
      }
    }
  }

  return { peers, readers, streams };
}

function declarationContent(
  event: Event,
  dTag: string,
): Record<string, unknown> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(event.content);
  } catch {
    throw new MalformedDeclarationError(dTag, event.id, "content is not JSON");
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new MalformedDeclarationError(
      dTag,
      event.id,
      "content must be a JSON object",
    );
  }
  return parsed as Record<string, unknown>;
}

function isActive(content: Record<string, unknown>): boolean {
  return (content.status ?? "active") === "active";
}

/** Active admit/read head → trust entry; revoked → null (claims domain only). */
function peerEntry(event: Event, dTag: string): ReplicationPeerTrust | null {
  const content = declarationContent(event, dTag);
  if (!isActive(content)) {
    return null;
  }
  const principal = content.principal;
  if (typeof principal !== "string" || principal === "") {
    throw new MalformedDeclarationError(
      dTag,
      event.id,
      "active head requires a string principal",
    );
  }
  const keys = event.tags
    .filter((tag) => tag[0] === "p" && typeof tag[1] === "string")
    .map((tag) => tag[1]);
  if (keys.length === 0) {
    throw new MalformedDeclarationError(
      dTag,
      event.id,
      "active head requires at least one p-tag verification key",
    );
  }
  return { principal, verification_keys: keys };
}

/** Active export head → normative selection; revoked → null. */
function exportEntry(event: Event, dTag: string): StreamExport | null {
  const content = declarationContent(event, dTag);
  if (!isActive(content)) {
    return null;
  }
  const selection = content.selection;
  if (typeof selection !== "object" || selection === null) {
    throw new MalformedDeclarationError(
      dTag,
      event.id,
      "active export head requires a selection object",
    );
  }
  const entry = selection as Record<string, unknown>;
  if (entry.mirror === true) {
    return { mode: "mirror" };
  }
  if (Array.isArray(entry.filter)) {
    return { mode: "filter", filters: entry.filter };
  }
  if (typeof entry.from_source === "string" && entry.from_source !== "") {
    return { mode: "from_source", source: entry.from_source };
  }
  throw new MalformedDeclarationError(
    dTag,
    event.id,
    'selection must be {mirror:true}, {filter:[...]}, or {from_source:"..."}',
  );
}
