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

/** A bounded, ordered page from this node's replication export. */
export interface ReplicationBatchWire {
  records: ReplicationRecordWire[];
  next_cursor: string;
  caught_up: boolean;
}

/** Read request for one exported stream. */
export interface ReplicationReadRequest {
  source: string;
  cursor: string | null;
  limit: number;
}

export const READ_CURSOR_PREFIX = "cf-sqlite-v1:";
export const MAX_READ_BATCH = 500;

/** Parses a read request from an untrusted body. */
export function replicationReadRequestFromUnknown(
  value: unknown,
): ReplicationReadRequest {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ProtocolInputError("replication read body must be an object");
  }
  const record = value as Record<string, unknown>;
  if (typeof record.source !== "string" || record.source === "") {
    throw new ProtocolInputError("replication read requires a source");
  }
  const cursor = record.cursor ?? null;
  if (cursor !== null && typeof cursor !== "string") {
    throw new ProtocolInputError("cursor must be a string or null");
  }
  if (
    cursor !== null &&
    (!cursor.startsWith(READ_CURSOR_PREFIX) ||
      !/^[0-9]+$/.test(cursor.slice(READ_CURSOR_PREFIX.length)))
  ) {
    throw new ProtocolInputError("cursor was not issued by this source");
  }
  const limit =
    typeof record.limit === "number" && Number.isSafeInteger(record.limit)
      ? record.limit
      : MAX_READ_BATCH;
  if (limit < 1) {
    throw new ProtocolInputError("limit must be at least 1");
  }
  return { source: record.source, cursor: cursor as string | null, limit };
}

/**
 * Parses exported stream declarations: `{"<source>": {"filter": null|[...]}}`.
 * A stream's filter is part of its identity — redefinitions require a new
 * stream name. Malformed configuration exports nothing (fail closed).
 */
export function parseStreamExports(
  raw: string | undefined,
): Record<string, unknown[] | null> {
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
  const streams: Record<string, unknown[] | null> = {};
  for (const [name, candidate] of Object.entries(parsed)) {
    if (
      typeof candidate !== "object" ||
      candidate === null ||
      !("filter" in (candidate as Record<string, unknown>))
    ) {
      continue;
    }
    const filter = (candidate as Record<string, unknown>).filter;
    if (filter === null) {
      streams[name] = null;
    } else if (Array.isArray(filter)) {
      streams[name] = filter;
    }
  }
  return streams;
}
