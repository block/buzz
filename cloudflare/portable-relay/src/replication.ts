// Replication wire types and destination-controlled peer trust.
//
// Wire shapes mirror the serde encoding of buzz-core's replication types:
// source IDs and cursors are transparent strings, and receipt outcomes are
// tagged with `status` in snake_case. Peer trust is deployment configuration
// (`BUZZ_REPLICATION_PEERS`), never inferred from request contents.

import type { Event } from "nostr-tools";
import { eventFromUnknown, ProtocolInputError } from "./protocol";

export const MAX_REPLICATION_BATCH_SIZE = 500;

/** One exact signed event exported from durable source history. */
export interface ReplicationRecordWire {
  source: string;
  cursor: string;
  event: Event;
}

export type ReplicationOutcomeWire =
  | { status: "stored" }
  | { status: "duplicate" }
  | { status: "superseded" }
  | { status: "rejected"; reason: string };

/** Destination acknowledgement bound to the source checkpoint. */
export interface ReplicationReceiptWire {
  source: string;
  cursor: string;
  event_id: string;
  outcome: ReplicationOutcomeWire;
}

/** Destination-controlled binding for one replication source stream. */
export interface ReplicationPeerTrust {
  principal: string;
  verification_keys: string[];
}

/** Parses records from an untrusted request body. */
export function replicationRecordsFromUnknown(
  value: unknown,
): ReplicationRecordWire[] {
  if (!Array.isArray(value)) {
    throw new ProtocolInputError("replication body must be a record array");
  }
  if (value.length > MAX_REPLICATION_BATCH_SIZE) {
    throw new ProtocolInputError(
      `replication batch exceeds ${MAX_REPLICATION_BATCH_SIZE} records`,
    );
  }
  return value.map((candidate) => {
    if (
      typeof candidate !== "object" ||
      candidate === null ||
      Array.isArray(candidate)
    ) {
      throw new ProtocolInputError("each replication record must be an object");
    }
    const record = candidate as Record<string, unknown>;
    if (typeof record.source !== "string" || record.source === "") {
      throw new ProtocolInputError("replication record requires a source");
    }
    if (typeof record.cursor !== "string" || record.cursor === "") {
      throw new ProtocolInputError("replication record requires a cursor");
    }
    return {
      source: record.source,
      cursor: record.cursor,
      event: eventFromUnknown(record.event),
    };
  });
}

/**
 * Parses destination peer trust from deployment configuration. Absent or
 * malformed configuration yields no trusted peers, which fails closed as
 * `peer_unbound` at the ingest boundary.
 */
export function parsePeerTrust(
  raw: string | undefined,
): Record<string, ReplicationPeerTrust> {
  if (raw === undefined || raw.trim() === "") {
    return {};
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return {};
  }
  const trust: Record<string, ReplicationPeerTrust> = {};
  for (const [source, candidate] of Object.entries(parsed)) {
    if (
      typeof candidate !== "object" ||
      candidate === null ||
      typeof (candidate as Record<string, unknown>).principal !== "string" ||
      !Array.isArray((candidate as Record<string, unknown>).verification_keys)
    ) {
      continue;
    }
    const keys = (
      (candidate as Record<string, unknown>).verification_keys as unknown[]
    ).filter((key): key is string => typeof key === "string");
    trust[source] = {
      principal: (candidate as Record<string, unknown>).principal as string,
      verification_keys: keys,
    };
  }
  return trust;
}
