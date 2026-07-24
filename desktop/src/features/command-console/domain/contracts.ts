import { isClassification, resolveClassification } from "./classification";
import type { Classification } from "./classification";

export const COMMAND_CONTRACT_VERSION = 1 as const;

export type JsonPrimitive = boolean | number | string | null;
export type JsonValue =
  | JsonPrimitive
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

type ClassifiedContract = {
  readonly version: typeof COMMAND_CONTRACT_VERSION;
  readonly classification: Classification;
};

export type SourceReference = ClassifiedContract & {
  readonly kind: "source-reference";
  readonly id: string;
  readonly title: string;
  readonly locator: string;
  readonly capturedAt: string;
};

export type AdviserContribution = ClassifiedContract & {
  readonly kind: "adviser-contribution";
  readonly id: string;
  readonly adviser: string;
  readonly summary: string;
  readonly sources: readonly SourceReference[];
  readonly producedAt: string;
};

export type CommandBrief = ClassifiedContract & {
  readonly kind: "command-brief";
  readonly id: string;
  readonly title: string;
  readonly summary: string;
  readonly contributions: readonly AdviserContribution[];
  readonly createdAt: string;
};

export type WorkspaceOperation = "create" | "update" | "delete";

export type ProposedWorkspaceAction = ClassifiedContract & {
  readonly kind: "proposed-workspace-action";
  readonly id: string;
  readonly operation: WorkspaceOperation;
  readonly target: string;
  readonly rationale: string;
  readonly proposedAt: string;
};

export type ModelRoute = ClassifiedContract & {
  readonly kind: "model-route";
  readonly id: string;
  readonly adviser: string;
  readonly provider: string;
  readonly model: string;
  readonly rationale: string;
  readonly selectedAt: string;
};

export type KnowledgeSnapshotManifest = ClassifiedContract & {
  readonly kind: "knowledge-snapshot-manifest";
  readonly id: string;
  readonly createdAt: string;
  readonly checksum: string;
  readonly sources: readonly SourceReference[];
};

export type MemoryRevision = ClassifiedContract & {
  readonly kind: "memory-revision";
  readonly id: string;
  readonly entityId: string;
  readonly revision: number;
  readonly revisedAt: string;
  readonly content: JsonValue;
};

export type ReplicatedPayload = KnowledgeSnapshotManifest | MemoryRevision;

export type ReplicationEnvelope = ClassifiedContract & {
  readonly kind: "replication-envelope";
  readonly id: string;
  readonly sequence: number;
  readonly createdAt: string;
  readonly payload: ReplicatedPayload;
};

export type SourceReferenceInput = {
  readonly id: string;
  readonly title: string;
  readonly locator: string;
  readonly capturedAt: string;
  readonly classification?: Classification;
};

export type AdviserContributionInput = {
  readonly id: string;
  readonly adviser: string;
  readonly summary: string;
  readonly sources: readonly SourceReference[];
  readonly producedAt: string;
  readonly classification?: Classification;
};

export type CommandBriefInput = {
  readonly id: string;
  readonly title: string;
  readonly summary: string;
  readonly contributions: readonly AdviserContribution[];
  readonly createdAt: string;
  readonly classification?: Classification;
};

export type ProposedWorkspaceActionInput = {
  readonly id: string;
  readonly operation: WorkspaceOperation;
  readonly target: string;
  readonly rationale: string;
  readonly proposedAt: string;
  readonly classification?: Classification;
};

export type ModelRouteInput = {
  readonly id: string;
  readonly adviser: string;
  readonly provider: string;
  readonly model: string;
  readonly rationale: string;
  readonly selectedAt: string;
  readonly classification?: Classification;
};

export type KnowledgeSnapshotManifestInput = {
  readonly id: string;
  readonly createdAt: string;
  readonly checksum: string;
  readonly sources: readonly SourceReference[];
  readonly classification?: Classification;
};

export type MemoryRevisionInput = {
  readonly id: string;
  readonly entityId: string;
  readonly revision: number;
  readonly revisedAt: string;
  readonly content: JsonValue;
  readonly classification?: Classification;
};

export type ReplicationEnvelopeInput = {
  readonly id: string;
  readonly sequence: number;
  readonly createdAt: string;
  readonly payload: ReplicatedPayload;
  readonly classification?: Classification;
};

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function hasExactKeys(value: UnknownRecord, keys: readonly string[]): boolean {
  const actualKeys = Object.keys(value);
  return (
    actualKeys.length === keys.length &&
    actualKeys.every((key) => keys.includes(key))
  );
}

function isText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isTimestamp(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    Number.isFinite(Date.parse(value))
  );
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}

function isPositiveSafeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 1;
}

function isWorkspaceOperation(value: unknown): value is WorkspaceOperation {
  return value === "create" || value === "update" || value === "delete";
}

function isJsonValue(value: unknown): value is JsonValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "boolean"
  ) {
    return true;
  }
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(isJsonValue);
  if (!isRecord(value)) return false;
  return Object.values(value).every(isJsonValue);
}

function cloneJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(cloneJson);
  if (isRecord(value)) {
    return Object.fromEntries(
      Object.entries(value).map(([key, nested]) => [
        key,
        cloneJson(nested as JsonValue),
      ]),
    );
  }
  return value;
}

function deepFreeze<T>(value: T): T {
  if (typeof value !== "object" || value === null || Object.isFrozen(value)) {
    return value;
  }
  for (const nested of Object.values(value)) {
    deepFreeze(nested);
  }
  return Object.freeze(value);
}

function requiredParsed<T>(parsed: T | null, kind: string): T {
  if (parsed === null) {
    throw new TypeError(`Invalid ${kind} contract input.`);
  }
  return parsed;
}

/**
 * Creates an immutable source reference, defaulting to `OFFICIAL`.
 */
export function createSourceReference(
  input: SourceReferenceInput,
): SourceReference {
  return requiredParsed(
    parseSourceReference({
      kind: "source-reference",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      title: input.title,
      locator: input.locator,
      capturedAt: input.capturedAt,
      classification: resolveClassification(input.classification),
    }),
    "source-reference",
  );
}

/**
 * Parses an exact persisted source-reference shape and returns a frozen copy.
 */
export function parseSourceReference(value: unknown): SourceReference | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "title",
      "locator",
      "capturedAt",
      "classification",
    ]) ||
    value.kind !== "source-reference" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isText(value.title) ||
    !isText(value.locator) ||
    !isTimestamp(value.capturedAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    title: value.title,
    locator: value.locator,
    capturedAt: value.capturedAt,
    classification: value.classification,
  });
}

/**
 * Creates an immutable adviser contribution and inherits source classification.
 */
export function createAdviserContribution(
  input: AdviserContributionInput,
): AdviserContribution {
  return requiredParsed(
    parseAdviserContribution({
      kind: "adviser-contribution",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      adviser: input.adviser,
      summary: input.summary,
      sources: input.sources,
      producedAt: input.producedAt,
      classification: resolveClassification(
        input.classification,
        input.sources.map((source) => source.classification),
      ),
    }),
    "adviser-contribution",
  );
}

/**
 * Parses an exact adviser-contribution shape and rejects classification loss.
 */
export function parseAdviserContribution(
  value: unknown,
): AdviserContribution | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "adviser",
      "summary",
      "sources",
      "producedAt",
      "classification",
    ]) ||
    value.kind !== "adviser-contribution" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isText(value.adviser) ||
    !isText(value.summary) ||
    !Array.isArray(value.sources) ||
    !isTimestamp(value.producedAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }
  const sources = value.sources.map(parseSourceReference);
  if (sources.some((source) => source === null)) return null;
  const parsedSources = sources as SourceReference[];
  if (
    resolveClassification(
      value.classification,
      parsedSources.map((source) => source.classification),
    ) !== value.classification
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    adviser: value.adviser,
    summary: value.summary,
    sources: parsedSources,
    producedAt: value.producedAt,
    classification: value.classification,
  });
}

/**
 * Creates an immutable command brief and inherits contribution classification.
 */
export function createCommandBrief(input: CommandBriefInput): CommandBrief {
  return requiredParsed(
    parseCommandBrief({
      kind: "command-brief",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      title: input.title,
      summary: input.summary,
      contributions: input.contributions,
      createdAt: input.createdAt,
      classification: resolveClassification(
        input.classification,
        input.contributions.map((contribution) => contribution.classification),
      ),
    }),
    "command-brief",
  );
}

/**
 * Parses an exact command-brief shape and rejects classification loss.
 */
export function parseCommandBrief(value: unknown): CommandBrief | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "title",
      "summary",
      "contributions",
      "createdAt",
      "classification",
    ]) ||
    value.kind !== "command-brief" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isText(value.title) ||
    !isText(value.summary) ||
    !Array.isArray(value.contributions) ||
    !isTimestamp(value.createdAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }
  const contributions = value.contributions.map(parseAdviserContribution);
  if (contributions.some((contribution) => contribution === null)) return null;
  const parsedContributions = contributions as AdviserContribution[];
  if (
    resolveClassification(
      value.classification,
      parsedContributions.map((contribution) => contribution.classification),
    ) !== value.classification
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    title: value.title,
    summary: value.summary,
    contributions: parsedContributions,
    createdAt: value.createdAt,
    classification: value.classification,
  });
}

/**
 * Creates an immutable proposal only; it does not execute workspace mutation.
 */
export function createProposedWorkspaceAction(
  input: ProposedWorkspaceActionInput,
): ProposedWorkspaceAction {
  return requiredParsed(
    parseProposedWorkspaceAction({
      kind: "proposed-workspace-action",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      operation: input.operation,
      target: input.target,
      rationale: input.rationale,
      proposedAt: input.proposedAt,
      classification: resolveClassification(input.classification),
    }),
    "proposed-workspace-action",
  );
}

/**
 * Parses an exact persisted workspace-action proposal.
 */
export function parseProposedWorkspaceAction(
  value: unknown,
): ProposedWorkspaceAction | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "operation",
      "target",
      "rationale",
      "proposedAt",
      "classification",
    ]) ||
    value.kind !== "proposed-workspace-action" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isWorkspaceOperation(value.operation) ||
    !isText(value.target) ||
    !isText(value.rationale) ||
    !isTimestamp(value.proposedAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    operation: value.operation,
    target: value.target,
    rationale: value.rationale,
    proposedAt: value.proposedAt,
    classification: value.classification,
  });
}

/**
 * Creates an immutable description of a selected model route.
 */
export function createModelRoute(input: ModelRouteInput): ModelRoute {
  return requiredParsed(
    parseModelRoute({
      kind: "model-route",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      adviser: input.adviser,
      provider: input.provider,
      model: input.model,
      rationale: input.rationale,
      selectedAt: input.selectedAt,
      classification: resolveClassification(input.classification),
    }),
    "model-route",
  );
}

/**
 * Parses an exact persisted model-route description.
 */
export function parseModelRoute(value: unknown): ModelRoute | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "adviser",
      "provider",
      "model",
      "rationale",
      "selectedAt",
      "classification",
    ]) ||
    value.kind !== "model-route" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isText(value.adviser) ||
    !isText(value.provider) ||
    !isText(value.model) ||
    !isText(value.rationale) ||
    !isTimestamp(value.selectedAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    adviser: value.adviser,
    provider: value.provider,
    model: value.model,
    rationale: value.rationale,
    selectedAt: value.selectedAt,
    classification: value.classification,
  });
}

/**
 * Creates an immutable knowledge manifest and inherits source classification.
 */
export function createKnowledgeSnapshotManifest(
  input: KnowledgeSnapshotManifestInput,
): KnowledgeSnapshotManifest {
  return requiredParsed(
    parseKnowledgeSnapshotManifest({
      kind: "knowledge-snapshot-manifest",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      createdAt: input.createdAt,
      checksum: input.checksum,
      sources: input.sources,
      classification: resolveClassification(
        input.classification,
        input.sources.map((source) => source.classification),
      ),
    }),
    "knowledge-snapshot-manifest",
  );
}

/**
 * Parses an exact knowledge manifest and rejects classification loss.
 */
export function parseKnowledgeSnapshotManifest(
  value: unknown,
): KnowledgeSnapshotManifest | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "createdAt",
      "checksum",
      "sources",
      "classification",
    ]) ||
    value.kind !== "knowledge-snapshot-manifest" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isTimestamp(value.createdAt) ||
    !isText(value.checksum) ||
    !Array.isArray(value.sources) ||
    !isClassification(value.classification)
  ) {
    return null;
  }
  const sources = value.sources.map(parseSourceReference);
  if (sources.some((source) => source === null)) return null;
  const parsedSources = sources as SourceReference[];
  if (
    resolveClassification(
      value.classification,
      parsedSources.map((source) => source.classification),
    ) !== value.classification
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    createdAt: value.createdAt,
    checksum: value.checksum,
    sources: parsedSources,
    classification: value.classification,
  });
}

/**
 * Creates an immutable JSON-only memory revision.
 */
export function createMemoryRevision(
  input: MemoryRevisionInput,
): MemoryRevision {
  return requiredParsed(
    parseMemoryRevision({
      kind: "memory-revision",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      entityId: input.entityId,
      revision: input.revision,
      revisedAt: input.revisedAt,
      content: input.content,
      classification: resolveClassification(input.classification),
    }),
    "memory-revision",
  );
}

/**
 * Parses an exact persisted memory revision with JSON-only content.
 */
export function parseMemoryRevision(value: unknown): MemoryRevision | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "entityId",
      "revision",
      "revisedAt",
      "content",
      "classification",
    ]) ||
    value.kind !== "memory-revision" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isText(value.entityId) ||
    !isPositiveSafeInteger(value.revision) ||
    !isTimestamp(value.revisedAt) ||
    !isJsonValue(value.content) ||
    !isClassification(value.classification)
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    entityId: value.entityId,
    revision: value.revision,
    revisedAt: value.revisedAt,
    content: cloneJson(value.content),
    classification: value.classification,
  });
}

function parseReplicatedPayload(value: unknown): ReplicatedPayload | null {
  if (!isRecord(value)) return null;
  if (value.kind === "knowledge-snapshot-manifest") {
    return parseKnowledgeSnapshotManifest(value);
  }
  if (value.kind === "memory-revision") return parseMemoryRevision(value);
  return null;
}

/**
 * Creates an immutable replication envelope and inherits payload classification.
 */
export function createReplicationEnvelope(
  input: ReplicationEnvelopeInput,
): ReplicationEnvelope {
  return requiredParsed(
    parseReplicationEnvelope({
      kind: "replication-envelope",
      version: COMMAND_CONTRACT_VERSION,
      id: input.id,
      sequence: input.sequence,
      createdAt: input.createdAt,
      payload: input.payload,
      classification: resolveClassification(input.classification, [
        input.payload.classification,
      ]),
    }),
    "replication-envelope",
  );
}

/**
 * Parses an exact replication envelope and rejects payload downgrades.
 */
export function parseReplicationEnvelope(
  value: unknown,
): ReplicationEnvelope | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "id",
      "sequence",
      "createdAt",
      "payload",
      "classification",
    ]) ||
    value.kind !== "replication-envelope" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isText(value.id) ||
    !isNonNegativeSafeInteger(value.sequence) ||
    !isTimestamp(value.createdAt) ||
    !isClassification(value.classification)
  ) {
    return null;
  }
  const payload = parseReplicatedPayload(value.payload);
  if (
    payload === null ||
    resolveClassification(value.classification, [payload.classification]) !==
      value.classification
  ) {
    return null;
  }

  return deepFreeze({
    kind: value.kind,
    version: value.version,
    id: value.id,
    sequence: value.sequence,
    createdAt: value.createdAt,
    payload,
    classification: value.classification,
  });
}
