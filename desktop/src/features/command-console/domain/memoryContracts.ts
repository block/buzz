import { isClassification, resolveClassification } from "./classification";
import type { Classification } from "./classification";
import {
  classificationIsSafe,
  cloneBoundedJson,
  hasExactKeys,
  isHash,
  isRecord,
  isRfc3339,
  isText,
  parseTextArray,
  required,
} from "./validation";
import type { JsonValue } from "./validation";

const MEMORY_CONTRACT_VERSION = 1 as const;

type MemoryContractBase = {
  readonly version: typeof MEMORY_CONTRACT_VERSION;
  readonly classification: Classification;
};

type ClassifiedInput<T> = Omit<T, "kind" | "version" | "classification"> & {
  readonly classification?: Classification;
};

export type MemoryHashes = {
  readonly content: string;
  readonly revision: string;
};

/** One lineage-addressed memory event, including explicit tombstones. */
export type MemoryRevision = MemoryContractBase & {
  readonly kind: "memory-revision";
  readonly entityId: string;
  readonly eventId: string;
  readonly parentRevisionIds: readonly string[];
  readonly nodeId: string;
  readonly timestamp: string;
  readonly hashes: MemoryHashes;
  readonly tombstone: boolean;
  readonly cursor: string;
  readonly content: JsonValue;
};

export type ReplicationHashes = {
  readonly payload: string;
  readonly envelope: string;
};

/** Resumable replication metadata around a memory revision. */
export type ReplicationEnvelope = MemoryContractBase & {
  readonly kind: "replication-envelope";
  readonly entityId: string;
  readonly eventId: string;
  readonly parentRevisionIds: readonly string[];
  readonly nodeId: string;
  readonly timestamp: string;
  readonly hashes: ReplicationHashes;
  readonly tombstone: boolean;
  readonly cursor: string;
  readonly payload: MemoryRevision;
};

function parseMemoryHashes(value: unknown): MemoryHashes | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["content", "revision"]) ||
    !isHash(value.content) ||
    !isHash(value.revision)
  )
    return null;
  return Object.freeze({ content: value.content, revision: value.revision });
}

/** Creates an immutable memory revision with bounded JSON content. */
export function createMemoryRevision(
  input: ClassifiedInput<MemoryRevision>,
): MemoryRevision {
  return required(
    parseMemoryRevision({
      kind: "memory-revision",
      version: MEMORY_CONTRACT_VERSION,
      classification: resolveClassification(input.classification),
      entityId: input.entityId,
      eventId: input.eventId,
      parentRevisionIds: input.parentRevisionIds,
      nodeId: input.nodeId,
      timestamp: input.timestamp,
      hashes: input.hashes,
      tombstone: input.tombstone,
      cursor: input.cursor,
      content: input.content,
    }),
    "memory-revision",
  );
}

/** Safely parses memory lineage and bounded JSON; never throws on bad content. */
export function parseMemoryRevision(value: unknown): MemoryRevision | null {
  try {
    if (
      !isRecord(value) ||
      !hasExactKeys(value, [
        "kind",
        "version",
        "classification",
        "entityId",
        "eventId",
        "parentRevisionIds",
        "nodeId",
        "timestamp",
        "hashes",
        "tombstone",
        "cursor",
        "content",
      ]) ||
      value.kind !== "memory-revision" ||
      value.version !== MEMORY_CONTRACT_VERSION ||
      !isClassification(value.classification) ||
      !isText(value.entityId) ||
      !isText(value.eventId) ||
      !isText(value.nodeId) ||
      !isRfc3339(value.timestamp) ||
      typeof value.tombstone !== "boolean" ||
      !isText(value.cursor) ||
      value.tombstone !== (value.content === null)
    ) {
      return null;
    }
    const parentRevisionIds = parseTextArray(value.parentRevisionIds);
    const hashes = parseMemoryHashes(value.hashes);
    const content = cloneBoundedJson(value.content);
    if (!parentRevisionIds || !hashes || !content.ok) return null;
    return Object.freeze({
      kind: value.kind,
      version: value.version,
      classification: value.classification,
      entityId: value.entityId,
      eventId: value.eventId,
      parentRevisionIds,
      nodeId: value.nodeId,
      timestamp: value.timestamp,
      hashes,
      tombstone: value.tombstone,
      cursor: value.cursor,
      content: content.value,
    });
  } catch {
    return null;
  }
}

function parseReplicationHashes(value: unknown): ReplicationHashes | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["payload", "envelope"]) ||
    !isHash(value.payload) ||
    !isHash(value.envelope)
  )
    return null;
  return Object.freeze({ payload: value.payload, envelope: value.envelope });
}

/** Creates an immutable resumable envelope with payload classification inheritance. */
export function createReplicationEnvelope(
  input: ClassifiedInput<ReplicationEnvelope>,
): ReplicationEnvelope {
  return required(
    parseReplicationEnvelope({
      kind: "replication-envelope",
      version: MEMORY_CONTRACT_VERSION,
      classification: resolveClassification(input.classification, [
        input.payload.classification,
      ]),
      entityId: input.entityId,
      eventId: input.eventId,
      parentRevisionIds: input.parentRevisionIds,
      nodeId: input.nodeId,
      timestamp: input.timestamp,
      hashes: input.hashes,
      tombstone: input.tombstone,
      cursor: input.cursor,
      payload: input.payload,
    }),
    "replication-envelope",
  );
}

/** Safely parses exact replication lineage and rejects inconsistent payloads. */
export function parseReplicationEnvelope(
  value: unknown,
): ReplicationEnvelope | null {
  try {
    if (
      !isRecord(value) ||
      !hasExactKeys(value, [
        "kind",
        "version",
        "classification",
        "entityId",
        "eventId",
        "parentRevisionIds",
        "nodeId",
        "timestamp",
        "hashes",
        "tombstone",
        "cursor",
        "payload",
      ]) ||
      value.kind !== "replication-envelope" ||
      value.version !== MEMORY_CONTRACT_VERSION ||
      !isClassification(value.classification) ||
      !isText(value.entityId) ||
      !isText(value.eventId) ||
      !isText(value.nodeId) ||
      !isRfc3339(value.timestamp) ||
      typeof value.tombstone !== "boolean" ||
      !isText(value.cursor)
    )
      return null;
    const parentRevisionIds = parseTextArray(value.parentRevisionIds);
    const hashes = parseReplicationHashes(value.hashes);
    const payload = parseMemoryRevision(value.payload);
    if (
      !parentRevisionIds ||
      !hashes ||
      !payload ||
      value.entityId !== payload.entityId ||
      value.nodeId !== payload.nodeId ||
      value.tombstone !== payload.tombstone ||
      !parentRevisionIds.includes(payload.eventId) ||
      hashes.payload !== payload.hashes.revision ||
      !classificationIsSafe(value.classification, [payload.classification])
    ) {
      return null;
    }
    return Object.freeze({
      kind: value.kind,
      version: value.version,
      classification: value.classification,
      entityId: value.entityId,
      eventId: value.eventId,
      parentRevisionIds,
      nodeId: value.nodeId,
      timestamp: value.timestamp,
      hashes,
      tombstone: value.tombstone,
      cursor: value.cursor,
      payload,
    });
  } catch {
    return null;
  }
}
