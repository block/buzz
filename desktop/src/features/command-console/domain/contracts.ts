import { isClassification, resolveClassification } from "./classification";
import type { Classification } from "./classification";
import {
  cloneBoundedJson,
  classificationIsSafe,
  hasExactKeys,
  isApprovalState,
  isCount,
  isHash,
  isRecord,
  isRfc3339,
  isText,
  parseActionDetail,
  parseObjectArray,
  parseQuotedLocation,
  parseTextArray,
  required,
} from "./validation";
import type { JsonValue, UnknownRecord } from "./validation";

export const COMMAND_CONTRACT_VERSION = 1 as const;

export type { JsonPrimitive, JsonValue } from "./validation";

type ContractBase = {
  readonly version: typeof COMMAND_CONTRACT_VERSION;
  readonly classification: Classification;
};

export type QuotedLocation = {
  readonly quote: string;
  readonly location: string;
};

/** A quoted source chunk pinned to a collection snapshot. */
export type SourceReference = ContractBase & {
  readonly kind: "source-reference";
  readonly sourceId: string;
  readonly collection: string;
  readonly documentId: string;
  readonly chunkId: string;
  readonly timestamp: string;
  readonly snapshotId: string;
  readonly quotedLocation: QuotedLocation;
};

export type ApprovalState = "pending" | "approved" | "rejected";

type WorkspaceActionBase = ContractBase & {
  readonly kind: "proposed-workspace-action";
  readonly actionId: string;
  readonly rationale: string;
  readonly approvalState: ApprovalState;
};

export type TaskWorkspaceAction = WorkspaceActionBase & {
  readonly actionType: "task";
  readonly task: {
    readonly title: string;
    readonly dueAt: string;
  };
};

export type CanvasChecklistWorkspaceAction = WorkspaceActionBase & {
  readonly actionType: "canvas-checklist-update";
  readonly update: {
    readonly canvasId: string;
    readonly checklistId: string;
    readonly itemId: string;
    readonly completed: boolean;
  };
};

export type ScheduledBriefWorkspaceAction = WorkspaceActionBase & {
  readonly actionType: "scheduled-brief";
  readonly schedule: {
    readonly briefId: string;
    readonly scheduledFor: string;
  };
};

export type DraftMessageWorkspaceAction = WorkspaceActionBase & {
  readonly actionType: "draft-message";
  readonly draft: {
    readonly channelId: string;
    readonly body: string;
  };
};

export type RoutingWorkspaceAction = WorkspaceActionBase & {
  readonly actionType: "routing-action";
  readonly route: {
    readonly adviser: string;
    readonly destination: string;
  };
};

/** The complete closed set of approval-gated workspace proposals. */
export type ProposedWorkspaceAction =
  | TaskWorkspaceAction
  | CanvasChecklistWorkspaceAction
  | ScheduledBriefWorkspaceAction
  | DraftMessageWorkspaceAction
  | RoutingWorkspaceAction;

/** Structured output from one command adviser. */
export type AdviserContribution = ContractBase & {
  readonly kind: "adviser-contribution";
  readonly adviser: string;
  readonly findings: readonly string[];
  readonly evidence: readonly SourceReference[];
  readonly confidence: number;
  readonly limitations: readonly string[];
  readonly dissent: readonly string[];
  readonly proposedActions: readonly ProposedWorkspaceAction[];
};

export type SourceFreshness = {
  readonly asOf: string;
  readonly staleSourceIds: readonly string[];
};

/** Consolidated command output with its generation audit identity. */
export type CommandBrief = ContractBase & {
  readonly kind: "command-brief";
  readonly contributions: readonly AdviserContribution[];
  readonly consolidatedPriorities: readonly string[];
  readonly decisions: readonly string[];
  readonly sourceFreshness: SourceFreshness;
  readonly generationAuditId: string;
};

export type ModelFallback = {
  readonly provider: string;
  readonly model: string;
};

export type EgressDecision = {
  readonly allowed: boolean;
  readonly rationale: string;
};

/** Auditable provider/model selection and its execution boundaries. */
export type ModelRoute = ContractBase & {
  readonly kind: "model-route";
  readonly selectedProvider: string;
  readonly selectedModel: string;
  readonly permittedTools: readonly string[];
  readonly fallbackChain: readonly ModelFallback[];
  readonly egressDecision: EgressDecision;
};

export type SnapshotHashes = {
  readonly manifest: string;
  readonly content: string;
};

export type CollectionSnapshot = {
  readonly collection: string;
  readonly schemaVersion: string;
  readonly documentCount: number;
  readonly chunkCount: number;
};

export type RetrievalModelVersion = {
  readonly model: string;
  readonly version: string;
};

/** Integrity and schema manifest for one replicated knowledge snapshot. */
export type KnowledgeSnapshotManifest = ContractBase & {
  readonly kind: "knowledge-snapshot-manifest";
  readonly snapshotId: string;
  readonly createdAt: string;
  readonly hashes: SnapshotHashes;
  readonly collections: readonly CollectionSnapshot[];
  readonly serviceRevision: string;
  readonly retrievalModelVersions: readonly RetrievalModelVersion[];
};

export type MemoryHashes = {
  readonly content: string;
  readonly revision: string;
};

/** One lineage-addressed memory event, including explicit tombstones. */
export type MemoryRevision = ContractBase & {
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
export type ReplicationEnvelope = ContractBase & {
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

type ClassifiedInput<T> = Omit<T, "kind" | "version" | "classification"> & {
  readonly classification?: Classification;
};
type ProposedWorkspaceActionInput =
  | ClassifiedInput<TaskWorkspaceAction>
  | ClassifiedInput<CanvasChecklistWorkspaceAction>
  | ClassifiedInput<ScheduledBriefWorkspaceAction>
  | ClassifiedInput<DraftMessageWorkspaceAction>
  | ClassifiedInput<RoutingWorkspaceAction>;

/** Creates an immutable source reference; omitted classification is OFFICIAL. */
export function createSourceReference(
  input: ClassifiedInput<SourceReference>,
): SourceReference {
  return required(
    parseSourceReference({
      kind: "source-reference",
      version: COMMAND_CONTRACT_VERSION,
      classification: resolveClassification(input.classification),
      sourceId: input.sourceId,
      collection: input.collection,
      documentId: input.documentId,
      chunkId: input.chunkId,
      timestamp: input.timestamp,
      snapshotId: input.snapshotId,
      quotedLocation: input.quotedLocation,
    }),
    "source-reference",
  );
}

/** Safely parses an exact persisted source reference. */
export function parseSourceReference(value: unknown): SourceReference | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "sourceId",
      "collection",
      "documentId",
      "chunkId",
      "timestamp",
      "snapshotId",
      "quotedLocation",
    ]) ||
    value.kind !== "source-reference" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isClassification(value.classification) ||
    !isText(value.sourceId) ||
    !isText(value.collection) ||
    !isText(value.documentId) ||
    !isText(value.chunkId) ||
    !isRfc3339(value.timestamp) ||
    !isText(value.snapshotId)
  )
    return null;
  const quotedLocation = parseQuotedLocation(value.quotedLocation);
  if (!quotedLocation) return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    sourceId: value.sourceId,
    collection: value.collection,
    documentId: value.documentId,
    chunkId: value.chunkId,
    timestamp: value.timestamp,
    snapshotId: value.snapshotId,
    quotedLocation,
  });
}

function actionBase(value: UnknownRecord): value is UnknownRecord & {
  kind: "proposed-workspace-action";
  version: typeof COMMAND_CONTRACT_VERSION;
  classification: Classification;
  actionId: string;
  rationale: string;
  approvalState: ApprovalState;
} {
  return (
    value.kind === "proposed-workspace-action" &&
    value.version === COMMAND_CONTRACT_VERSION &&
    isClassification(value.classification) &&
    isText(value.actionId) &&
    isText(value.rationale) &&
    isApprovalState(value.approvalState)
  );
}

/** Creates one of the five immutable, approval-gated proposal variants. */
export function createProposedWorkspaceAction(
  input: ProposedWorkspaceActionInput,
): ProposedWorkspaceAction {
  const common = {
    kind: "proposed-workspace-action",
    version: COMMAND_CONTRACT_VERSION,
    classification: resolveClassification(input.classification),
    actionType: input.actionType,
    actionId: input.actionId,
    rationale: input.rationale,
    approvalState: input.approvalState,
  } as const;
  const candidate =
    input.actionType === "task"
      ? { ...common, task: input.task }
      : input.actionType === "canvas-checklist-update"
        ? { ...common, update: input.update }
        : input.actionType === "scheduled-brief"
          ? { ...common, schedule: input.schedule }
          : input.actionType === "draft-message"
            ? { ...common, draft: input.draft }
            : { ...common, route: input.route };
  return required(
    parseProposedWorkspaceAction(candidate),
    "proposed-workspace-action",
  );
}

/** Safely parses the closed proposed-action union. */
export function parseProposedWorkspaceAction(
  value: unknown,
): ProposedWorkspaceAction | null {
  if (!isRecord(value) || !actionBase(value)) return null;
  const common = {
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    actionType: value.actionType,
    actionId: value.actionId,
    rationale: value.rationale,
    approvalState: value.approvalState,
  };
  if (value.actionType === "task") {
    if (!hasExactKeys(value, [...Object.keys(common), "task"])) return null;
    const task = parseActionDetail(value.task, ["title", "dueAt"]);
    if (!task || !isText(task.title) || !isRfc3339(task.dueAt)) return null;
    return Object.freeze({
      ...common,
      actionType: value.actionType,
      task: Object.freeze({ title: task.title, dueAt: task.dueAt }),
    });
  }
  if (value.actionType === "canvas-checklist-update") {
    if (!hasExactKeys(value, [...Object.keys(common), "update"])) return null;
    const update = parseActionDetail(value.update, [
      "canvasId",
      "checklistId",
      "itemId",
      "completed",
    ]);
    if (
      !update ||
      !isText(update.canvasId) ||
      !isText(update.checklistId) ||
      !isText(update.itemId) ||
      typeof update.completed !== "boolean"
    )
      return null;
    return Object.freeze({
      ...common,
      actionType: value.actionType,
      update: Object.freeze({
        canvasId: update.canvasId,
        checklistId: update.checklistId,
        itemId: update.itemId,
        completed: update.completed,
      }),
    });
  }
  if (value.actionType === "scheduled-brief") {
    if (!hasExactKeys(value, [...Object.keys(common), "schedule"])) return null;
    const schedule = parseActionDetail(value.schedule, [
      "briefId",
      "scheduledFor",
    ]);
    if (
      !schedule ||
      !isText(schedule.briefId) ||
      !isRfc3339(schedule.scheduledFor)
    )
      return null;
    return Object.freeze({
      ...common,
      actionType: value.actionType,
      schedule: Object.freeze({
        briefId: schedule.briefId,
        scheduledFor: schedule.scheduledFor,
      }),
    });
  }
  if (value.actionType === "draft-message") {
    if (!hasExactKeys(value, [...Object.keys(common), "draft"])) return null;
    const draft = parseActionDetail(value.draft, ["channelId", "body"]);
    if (!draft || !isText(draft.channelId) || !isText(draft.body)) return null;
    return Object.freeze({
      ...common,
      actionType: value.actionType,
      draft: Object.freeze({ channelId: draft.channelId, body: draft.body }),
    });
  }
  if (value.actionType === "routing-action") {
    if (!hasExactKeys(value, [...Object.keys(common), "route"])) return null;
    const route = parseActionDetail(value.route, ["adviser", "destination"]);
    if (!route || !isText(route.adviser) || !isText(route.destination))
      return null;
    return Object.freeze({
      ...common,
      actionType: value.actionType,
      route: Object.freeze({
        adviser: route.adviser,
        destination: route.destination,
      }),
    });
  }
  return null;
}

/** Creates immutable structured adviser output with classification inheritance. */
export function createAdviserContribution(
  input: ClassifiedInput<AdviserContribution>,
): AdviserContribution {
  return required(
    parseAdviserContribution({
      kind: "adviser-contribution",
      version: COMMAND_CONTRACT_VERSION,
      classification: resolveClassification(input.classification, [
        ...input.evidence.map((item) => item.classification),
        ...input.proposedActions.map((item) => item.classification),
      ]),
      adviser: input.adviser,
      findings: input.findings,
      evidence: input.evidence,
      confidence: input.confidence,
      limitations: input.limitations,
      dissent: input.dissent,
      proposedActions: input.proposedActions,
    }),
    "adviser-contribution",
  );
}

/** Safely parses exact structured adviser output. */
export function parseAdviserContribution(
  value: unknown,
): AdviserContribution | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "adviser",
      "findings",
      "evidence",
      "confidence",
      "limitations",
      "dissent",
      "proposedActions",
    ]) ||
    value.kind !== "adviser-contribution" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isClassification(value.classification) ||
    !isText(value.adviser) ||
    typeof value.confidence !== "number" ||
    !Number.isFinite(value.confidence) ||
    value.confidence < 0 ||
    value.confidence > 1
  )
    return null;
  const findings = parseTextArray(value.findings);
  const evidence = parseObjectArray(value.evidence, parseSourceReference);
  const limitations = parseTextArray(value.limitations);
  const dissent = parseTextArray(value.dissent);
  const proposedActions = parseObjectArray(
    value.proposedActions,
    parseProposedWorkspaceAction,
  );
  if (
    !findings ||
    !evidence ||
    !limitations ||
    !dissent ||
    !proposedActions ||
    !classificationIsSafe(value.classification, [
      ...evidence.map((item) => item.classification),
      ...proposedActions.map((item) => item.classification),
    ])
  )
    return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    adviser: value.adviser,
    findings,
    evidence,
    confidence: value.confidence,
    limitations,
    dissent,
    proposedActions,
  });
}

function parseSourceFreshness(value: unknown): SourceFreshness | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["asOf", "staleSourceIds"]) ||
    !isRfc3339(value.asOf)
  )
    return null;
  const staleSourceIds = parseTextArray(value.staleSourceIds);
  return staleSourceIds
    ? Object.freeze({ asOf: value.asOf, staleSourceIds })
    : null;
}

/** Creates an immutable consolidated brief with source/audit metadata. */
export function createCommandBrief(
  input: ClassifiedInput<CommandBrief>,
): CommandBrief {
  return required(
    parseCommandBrief({
      kind: "command-brief",
      version: COMMAND_CONTRACT_VERSION,
      classification: resolveClassification(
        input.classification,
        input.contributions.map((item) => item.classification),
      ),
      contributions: input.contributions,
      consolidatedPriorities: input.consolidatedPriorities,
      decisions: input.decisions,
      sourceFreshness: input.sourceFreshness,
      generationAuditId: input.generationAuditId,
    }),
    "command-brief",
  );
}

/** Safely parses an exact command brief. */
export function parseCommandBrief(value: unknown): CommandBrief | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "contributions",
      "consolidatedPriorities",
      "decisions",
      "sourceFreshness",
      "generationAuditId",
    ]) ||
    value.kind !== "command-brief" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isClassification(value.classification) ||
    !isText(value.generationAuditId)
  )
    return null;
  const contributions = parseObjectArray(
    value.contributions,
    parseAdviserContribution,
  );
  const consolidatedPriorities = parseTextArray(value.consolidatedPriorities);
  const decisions = parseTextArray(value.decisions);
  const sourceFreshness = parseSourceFreshness(value.sourceFreshness);
  if (
    !contributions ||
    !consolidatedPriorities ||
    !decisions ||
    !sourceFreshness ||
    !classificationIsSafe(
      value.classification,
      contributions.map((item) => item.classification),
    )
  )
    return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    contributions,
    consolidatedPriorities,
    decisions,
    sourceFreshness,
    generationAuditId: value.generationAuditId,
  });
}

function parseModelFallback(value: unknown): ModelFallback | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["provider", "model"]) ||
    !isText(value.provider) ||
    !isText(value.model)
  )
    return null;
  return Object.freeze({ provider: value.provider, model: value.model });
}

function parseEgressDecision(value: unknown): EgressDecision | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["allowed", "rationale"]) ||
    typeof value.allowed !== "boolean" ||
    !isText(value.rationale)
  )
    return null;
  return Object.freeze({ allowed: value.allowed, rationale: value.rationale });
}

/** Creates an immutable, auditable model route. */
export function createModelRoute(
  input: ClassifiedInput<ModelRoute>,
): ModelRoute {
  return required(
    parseModelRoute({
      kind: "model-route",
      version: COMMAND_CONTRACT_VERSION,
      classification: resolveClassification(input.classification),
      selectedProvider: input.selectedProvider,
      selectedModel: input.selectedModel,
      permittedTools: input.permittedTools,
      fallbackChain: input.fallbackChain,
      egressDecision: input.egressDecision,
    }),
    "model-route",
  );
}

/** Safely parses an exact model route. */
export function parseModelRoute(value: unknown): ModelRoute | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "selectedProvider",
      "selectedModel",
      "permittedTools",
      "fallbackChain",
      "egressDecision",
    ]) ||
    value.kind !== "model-route" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isClassification(value.classification) ||
    !isText(value.selectedProvider) ||
    !isText(value.selectedModel)
  )
    return null;
  const permittedTools = parseTextArray(value.permittedTools);
  const fallbackChain = parseObjectArray(
    value.fallbackChain,
    parseModelFallback,
  );
  const egressDecision = parseEgressDecision(value.egressDecision);
  if (!permittedTools || !fallbackChain || !egressDecision) return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    selectedProvider: value.selectedProvider,
    selectedModel: value.selectedModel,
    permittedTools,
    fallbackChain,
    egressDecision,
  });
}

function parseSnapshotHashes(value: unknown): SnapshotHashes | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["manifest", "content"]) ||
    !isHash(value.manifest) ||
    !isHash(value.content)
  )
    return null;
  return Object.freeze({ manifest: value.manifest, content: value.content });
}

function parseCollectionSnapshot(value: unknown): CollectionSnapshot | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "collection",
      "schemaVersion",
      "documentCount",
      "chunkCount",
    ]) ||
    !isText(value.collection) ||
    !isText(value.schemaVersion) ||
    !isCount(value.documentCount) ||
    !isCount(value.chunkCount)
  )
    return null;
  return Object.freeze({
    collection: value.collection,
    schemaVersion: value.schemaVersion,
    documentCount: value.documentCount,
    chunkCount: value.chunkCount,
  });
}

function parseRetrievalModelVersion(
  value: unknown,
): RetrievalModelVersion | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["model", "version"]) ||
    !isText(value.model) ||
    !isText(value.version)
  )
    return null;
  return Object.freeze({ model: value.model, version: value.version });
}

/** Creates an immutable knowledge snapshot manifest. */
export function createKnowledgeSnapshotManifest(
  input: ClassifiedInput<KnowledgeSnapshotManifest>,
): KnowledgeSnapshotManifest {
  return required(
    parseKnowledgeSnapshotManifest({
      kind: "knowledge-snapshot-manifest",
      version: COMMAND_CONTRACT_VERSION,
      classification: resolveClassification(input.classification),
      snapshotId: input.snapshotId,
      createdAt: input.createdAt,
      hashes: input.hashes,
      collections: input.collections,
      serviceRevision: input.serviceRevision,
      retrievalModelVersions: input.retrievalModelVersions,
    }),
    "knowledge-snapshot-manifest",
  );
}

/** Safely parses an exact knowledge snapshot manifest. */
export function parseKnowledgeSnapshotManifest(
  value: unknown,
): KnowledgeSnapshotManifest | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "kind",
      "version",
      "classification",
      "snapshotId",
      "createdAt",
      "hashes",
      "collections",
      "serviceRevision",
      "retrievalModelVersions",
    ]) ||
    value.kind !== "knowledge-snapshot-manifest" ||
    value.version !== COMMAND_CONTRACT_VERSION ||
    !isClassification(value.classification) ||
    !isText(value.snapshotId) ||
    !isRfc3339(value.createdAt) ||
    !isText(value.serviceRevision)
  )
    return null;
  const hashes = parseSnapshotHashes(value.hashes);
  const collections = parseObjectArray(
    value.collections,
    parseCollectionSnapshot,
  );
  const retrievalModelVersions = parseObjectArray(
    value.retrievalModelVersions,
    parseRetrievalModelVersion,
  );
  if (!hashes || !collections || !retrievalModelVersions) return null;
  return Object.freeze({
    kind: value.kind,
    version: value.version,
    classification: value.classification,
    snapshotId: value.snapshotId,
    createdAt: value.createdAt,
    hashes,
    collections,
    serviceRevision: value.serviceRevision,
    retrievalModelVersions,
  });
}

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
      version: COMMAND_CONTRACT_VERSION,
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
      value.version !== COMMAND_CONTRACT_VERSION ||
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
      version: COMMAND_CONTRACT_VERSION,
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
      value.version !== COMMAND_CONTRACT_VERSION ||
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
