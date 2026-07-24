import { isClassification, resolveClassification } from "./classification";
import type { Classification } from "./classification";
import {
  hasExactKeys,
  isCount,
  isRecord,
  isRfc3339,
  isText,
  parseObjectArray,
  parseTextArray,
  required,
} from "./validation";

const KNOWLEDGE_STATUS_VERSION = 1 as const;

type KnowledgeContractBase = {
  readonly version: typeof KNOWLEDGE_STATUS_VERSION;
  readonly classification: Classification;
};

type ClassifiedInput<T> = Omit<T, "kind" | "version" | "classification"> & {
  readonly classification?: Classification;
};

export type KnowledgeServiceStatus = "ready" | "not_configured" | "unavailable";
export type KnowledgeFreshness = "fresh" | "stale" | "unknown";
export type KnowledgeValidation = "verified" | "failed" | "unknown";

export type MemoryKnowledgeStatus = {
  readonly status: KnowledgeServiceStatus;
  readonly serverIdentity: string | null;
  readonly nodeId: string | null;
  readonly revisionCount: number;
  readonly conflictCount: number;
  readonly replicationCursor: number | null;
  readonly lastSuccessfulSync: string | null;
  readonly freshness: KnowledgeFreshness;
  readonly validation: KnowledgeValidation;
  readonly toolAllowlist: readonly string[];
  readonly error: string | null;
};

export type RagKnowledgeStatus = {
  readonly status: KnowledgeServiceStatus;
  readonly serverIdentity: string | null;
  readonly activeSnapshotId: string | null;
  readonly signatureFingerprint: string | null;
  readonly snapshotTime: string | null;
  readonly lastSuccessfulActivation: string | null;
  readonly freshness: KnowledgeFreshness;
  readonly validation: KnowledgeValidation;
  readonly toolAllowlist: readonly string[];
  readonly error: string | null;
};

export type AppleKnowledgeStatus = {
  readonly source: "calendar" | "reminders" | "notes" | "files";
  readonly permission:
    | "not_determined"
    | "denied"
    | "authorized"
    | "restricted"
    | "unavailable";
  readonly observedAt: string;
  readonly recordCount: number;
  readonly truncated: boolean;
  readonly error: string | null;
};

/** Metadata-only local knowledge readiness; never carries evidence or secrets. */
export type CommandKnowledgeStatus = KnowledgeContractBase & {
  readonly kind: "command-knowledge-status";
  readonly observedAt: string;
  readonly memory: MemoryKnowledgeStatus;
  readonly rag: RagKnowledgeStatus;
  readonly appleInputs: readonly AppleKnowledgeStatus[];
  readonly degradedSections: readonly string[];
};

function nullableText(value: unknown): value is string | null {
  return value === null || isText(value);
}

function nullableTimestamp(value: unknown): value is string | null {
  return value === null || isRfc3339(value);
}

function isKnowledgeServiceStatus(
  value: unknown,
): value is KnowledgeServiceStatus {
  return (
    value === "ready" || value === "not_configured" || value === "unavailable"
  );
}

function isKnowledgeFreshness(value: unknown): value is KnowledgeFreshness {
  return value === "fresh" || value === "stale" || value === "unknown";
}

function isKnowledgeValidation(value: unknown): value is KnowledgeValidation {
  return value === "verified" || value === "failed" || value === "unknown";
}

function parseToolAllowlist(value: unknown): readonly string[] | null {
  const tools = parseTextArray(value);
  if (
    !tools ||
    tools.length > 32 ||
    new Set(tools).size !== tools.length ||
    tools.some((tool) => !/^[A-Za-z0-9_]{1,128}$/.test(tool))
  )
    return null;
  return tools;
}

function isSha256Digest(value: string | null): value is string {
  return value !== null && /^[0-9a-f]{64}$/.test(value);
}

function parseMemoryKnowledgeStatus(
  value: unknown,
): MemoryKnowledgeStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "status",
      "serverIdentity",
      "nodeId",
      "revisionCount",
      "conflictCount",
      "replicationCursor",
      "lastSuccessfulSync",
      "freshness",
      "validation",
      "toolAllowlist",
      "error",
    ]) ||
    !isKnowledgeServiceStatus(value.status) ||
    !nullableText(value.serverIdentity) ||
    !nullableText(value.nodeId) ||
    !isCount(value.revisionCount) ||
    !isCount(value.conflictCount) ||
    !(value.replicationCursor === null || isCount(value.replicationCursor)) ||
    !nullableTimestamp(value.lastSuccessfulSync) ||
    !isKnowledgeFreshness(value.freshness) ||
    !isKnowledgeValidation(value.validation) ||
    !nullableText(value.error)
  )
    return null;
  const toolAllowlist = parseToolAllowlist(value.toolAllowlist);
  if (
    !toolAllowlist ||
    (value.status === "ready"
      ? value.serverIdentity !== "memory" ||
        value.nodeId === null ||
        value.freshness !== "fresh" ||
        value.validation !== "verified" ||
        value.error !== null
      : value.serverIdentity !== null || value.validation === "verified")
  )
    return null;
  return Object.freeze({
    status: value.status,
    serverIdentity: value.serverIdentity,
    nodeId: value.nodeId,
    revisionCount: value.revisionCount,
    conflictCount: value.conflictCount,
    replicationCursor: value.replicationCursor,
    lastSuccessfulSync: value.lastSuccessfulSync,
    freshness: value.freshness,
    validation: value.validation,
    toolAllowlist,
    error: value.error,
  });
}

function parseRagKnowledgeStatus(value: unknown): RagKnowledgeStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "status",
      "serverIdentity",
      "activeSnapshotId",
      "signatureFingerprint",
      "snapshotTime",
      "lastSuccessfulActivation",
      "freshness",
      "validation",
      "toolAllowlist",
      "error",
    ]) ||
    !isKnowledgeServiceStatus(value.status) ||
    !nullableText(value.serverIdentity) ||
    !nullableText(value.activeSnapshotId) ||
    !nullableText(value.signatureFingerprint) ||
    !nullableTimestamp(value.snapshotTime) ||
    !nullableTimestamp(value.lastSuccessfulActivation) ||
    !isKnowledgeFreshness(value.freshness) ||
    !isKnowledgeValidation(value.validation) ||
    !nullableText(value.error)
  )
    return null;
  const toolAllowlist = parseToolAllowlist(value.toolAllowlist);
  if (
    !toolAllowlist ||
    (value.status === "ready"
      ? value.serverIdentity !== "rag" ||
        !isSha256Digest(value.activeSnapshotId) ||
        !isSha256Digest(value.signatureFingerprint) ||
        value.snapshotTime === null ||
        value.lastSuccessfulActivation === null ||
        value.freshness !== "fresh" ||
        value.validation !== "verified" ||
        value.error !== null
      : value.serverIdentity !== null ||
        value.activeSnapshotId !== null ||
        value.signatureFingerprint !== null ||
        value.snapshotTime !== null ||
        value.lastSuccessfulActivation !== null ||
        value.validation === "verified")
  )
    return null;
  return Object.freeze({
    status: value.status,
    serverIdentity: value.serverIdentity,
    activeSnapshotId: value.activeSnapshotId,
    signatureFingerprint: value.signatureFingerprint,
    snapshotTime: value.snapshotTime,
    lastSuccessfulActivation: value.lastSuccessfulActivation,
    freshness: value.freshness,
    validation: value.validation,
    toolAllowlist,
    error: value.error,
  });
}

function parseAppleKnowledgeStatus(
  value: unknown,
): AppleKnowledgeStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "source",
      "permission",
      "observedAt",
      "recordCount",
      "truncated",
      "error",
    ]) ||
    !["calendar", "reminders", "notes", "files"].includes(
      String(value.source),
    ) ||
    ![
      "not_determined",
      "denied",
      "authorized",
      "restricted",
      "unavailable",
    ].includes(String(value.permission)) ||
    !isRfc3339(value.observedAt) ||
    !isCount(value.recordCount) ||
    typeof value.truncated !== "boolean" ||
    !nullableText(value.error)
  )
    return null;
  return Object.freeze({
    source: value.source as AppleKnowledgeStatus["source"],
    permission: value.permission as AppleKnowledgeStatus["permission"],
    observedAt: value.observedAt,
    recordCount: value.recordCount,
    truncated: value.truncated,
    error: value.error,
  });
}

function expectedDegradedSections(
  memory: MemoryKnowledgeStatus,
  rag: RagKnowledgeStatus,
  appleInputs: readonly AppleKnowledgeStatus[],
): readonly string[] {
  const sections: string[] = [];
  if (memory.status !== "ready") sections.push("memory-readiness");
  if (memory.conflictCount > 0) sections.push("memory-conflicts");
  if (rag.status !== "ready") sections.push("rag-readiness");
  for (const source of appleInputs) {
    if (source.permission !== "authorized" || source.error !== null) {
      sections.push(`apple-${source.source}`);
    }
  }
  return sections.sort();
}

/** Creates an immutable metadata-only command knowledge readiness record. */
export function createCommandKnowledgeStatus(
  input: ClassifiedInput<CommandKnowledgeStatus>,
): CommandKnowledgeStatus {
  return required(
    parseCommandKnowledgeStatus({
      kind: "command-knowledge-status",
      version: KNOWLEDGE_STATUS_VERSION,
      classification: resolveClassification(input.classification),
      observedAt: input.observedAt,
      memory: input.memory,
      rag: input.rag,
      appleInputs: input.appleInputs,
      degradedSections: input.degradedSections,
    }),
    "command-knowledge-status",
  );
}

/**
 * Validates the exact display/persistence shape. Cryptographic proof is
 * performed only in Rust; this parser never upgrades an asserted status.
 */
export function parseCommandKnowledgeStatus(
  value: unknown,
): CommandKnowledgeStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "observedAt",
      "memory",
      "rag",
      "appleInputs",
      "degradedSections",
    ]) ||
    value.kind !== "command-knowledge-status" ||
    value.version !== KNOWLEDGE_STATUS_VERSION ||
    !isClassification(value.classification) ||
    !isRfc3339(value.observedAt)
  )
    return null;
  const memory = parseMemoryKnowledgeStatus(value.memory);
  const rag = parseRagKnowledgeStatus(value.rag);
  const appleInputs = parseObjectArray(
    value.appleInputs,
    parseAppleKnowledgeStatus,
  );
  const degradedSections = parseTextArray(value.degradedSections);
  if (
    !memory ||
    !rag ||
    !appleInputs ||
    appleInputs.length !== 4 ||
    new Set(appleInputs.map((source) => source.source)).size !==
      appleInputs.length ||
    !degradedSections ||
    degradedSections.length > 16 ||
    new Set(degradedSections).size !== degradedSections.length ||
    JSON.stringify(degradedSections) !==
      JSON.stringify(expectedDegradedSections(memory, rag, appleInputs))
  )
    return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    observedAt: value.observedAt,
    memory,
    rag,
    appleInputs,
    degradedSections,
  });
}
