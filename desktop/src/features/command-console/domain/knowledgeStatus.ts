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
const TRUSTED_LAN_KNOWLEDGE_STATUS_VERSION = 2 as const;

type KnowledgeContractBase = {
  readonly version:
    | typeof KNOWLEDGE_STATUS_VERSION
    | typeof TRUSTED_LAN_KNOWLEDGE_STATUS_VERSION;
  readonly classification: Classification;
};

type ClassifiedInput<T> = Omit<T, "kind" | "version" | "classification"> & {
  readonly classification?: Classification;
};

export type KnowledgeServiceStatus = "ready" | "not_configured" | "unavailable";
export type KnowledgeFreshness =
  | "never_synced"
  | "fresh"
  | "stale"
  | "corrupt"
  | "observed"
  | "unknown";
export type KnowledgeValidation =
  | "verified"
  | "trusted_lan_observed"
  | "failed"
  | "unknown";

export type MemoryKnowledgeStatus = {
  readonly status: KnowledgeServiceStatus;
  readonly serverIdentity: string | null;
  readonly nodeId: string | null;
  readonly homeNodeId: string | null;
  readonly revisionCount: number;
  readonly conflictCount: number;
  readonly replicationCursor: number | null;
  readonly homeReplicationCursor: number | null;
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
  readonly sourceMode?: "trusted_lan";
  readonly modelRoute?: "local_litellm_openai";
  readonly evidenceAssurance?: "trusted_lan_observed";
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
  return (
    value === "never_synced" ||
    value === "fresh" ||
    value === "stale" ||
    value === "corrupt" ||
    value === "observed" ||
    value === "unknown"
  );
}

function isKnowledgeValidation(value: unknown): value is KnowledgeValidation {
  return (
    value === "verified" ||
    value === "trusted_lan_observed" ||
    value === "failed" ||
    value === "unknown"
  );
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
  trustedLan = false,
): MemoryKnowledgeStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "status",
      "serverIdentity",
      "nodeId",
      "homeNodeId",
      "revisionCount",
      "conflictCount",
      "replicationCursor",
      "homeReplicationCursor",
      "lastSuccessfulSync",
      "freshness",
      "validation",
      "toolAllowlist",
      "error",
    ]) ||
    !isKnowledgeServiceStatus(value.status) ||
    !nullableText(value.serverIdentity) ||
    !nullableText(value.nodeId) ||
    !nullableText(value.homeNodeId) ||
    !isCount(value.revisionCount) ||
    !isCount(value.conflictCount) ||
    !(value.replicationCursor === null || isCount(value.replicationCursor)) ||
    !(
      value.homeReplicationCursor === null ||
      isCount(value.homeReplicationCursor)
    ) ||
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
        (trustedLan
          ? value.nodeId !== null ||
            value.freshness !== "observed" ||
            value.validation !== "trusted_lan_observed"
          : value.nodeId === null ||
            value.freshness === "unknown" ||
            value.validation !== "verified") ||
        value.error !== null
      : value.serverIdentity !== null || value.validation === "verified")
  )
    return null;
  return Object.freeze({
    status: value.status,
    serverIdentity: value.serverIdentity,
    nodeId: value.nodeId,
    homeNodeId: value.homeNodeId,
    revisionCount: value.revisionCount,
    conflictCount: value.conflictCount,
    replicationCursor: value.replicationCursor,
    homeReplicationCursor: value.homeReplicationCursor,
    lastSuccessfulSync: value.lastSuccessfulSync,
    freshness: value.freshness,
    validation: value.validation,
    toolAllowlist,
    error: value.error,
  });
}

function parseRagKnowledgeStatus(
  value: unknown,
  trustedLan = false,
): RagKnowledgeStatus | null {
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
        (trustedLan
          ? value.signatureFingerprint !== null ||
            value.freshness !== "observed" ||
            value.validation !== "trusted_lan_observed"
          : !isSha256Digest(value.signatureFingerprint) ||
            value.freshness !== "fresh" ||
            value.validation !== "verified") ||
        value.snapshotTime === null ||
        value.lastSuccessfulActivation === null ||
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
  trustedLan = false,
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
  if (trustedLan) sections.push("trusted-lan-unsigned");
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
  const trustedLan =
    isRecord(value) && value.version === TRUSTED_LAN_KNOWLEDGE_STATUS_VERSION;
  const exactKeys = trustedLan
    ? [
        "kind",
        "version",
        "classification",
        "sourceMode",
        "modelRoute",
        "evidenceAssurance",
        "observedAt",
        "memory",
        "rag",
        "appleInputs",
        "degradedSections",
      ]
    : [
        "kind",
        "version",
        "classification",
        "observedAt",
        "memory",
        "rag",
        "appleInputs",
        "degradedSections",
      ];
  if (
    !isRecord(value) ||
    !hasExactKeys(value, exactKeys) ||
    value.kind !== "command-knowledge-status" ||
    (value.version !== KNOWLEDGE_STATUS_VERSION && !trustedLan) ||
    (trustedLan &&
      (value.sourceMode !== "trusted_lan" ||
        value.modelRoute !== "local_litellm_openai" ||
        value.evidenceAssurance !== "trusted_lan_observed")) ||
    !isClassification(value.classification) ||
    !isRfc3339(value.observedAt)
  )
    return null;
  const memory = parseMemoryKnowledgeStatus(value.memory, trustedLan);
  const rag = parseRagKnowledgeStatus(value.rag, trustedLan);
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
      JSON.stringify(
        expectedDegradedSections(memory, rag, appleInputs, trustedLan),
      )
  )
    return null;
  const parsed = {
    kind: "command-knowledge-status" as const,
    version: trustedLan
      ? TRUSTED_LAN_KNOWLEDGE_STATUS_VERSION
      : KNOWLEDGE_STATUS_VERSION,
    classification: value.classification,
    observedAt: value.observedAt,
    memory,
    rag,
    appleInputs,
    degradedSections,
    ...(trustedLan
      ? {
          sourceMode: "trusted_lan" as const,
          modelRoute: "local_litellm_openai" as const,
          evidenceAssurance: "trusted_lan_observed" as const,
        }
      : {}),
  };
  return Object.freeze(parsed);
}
