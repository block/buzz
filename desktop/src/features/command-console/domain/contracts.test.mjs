import assert from "node:assert/strict";
import test from "node:test";

import {
  createAdviserContribution,
  createCommandBrief,
  createKnowledgeSnapshotManifest,
  createMemoryRevision,
  createModelRoute,
  createProposedWorkspaceAction,
  createReplicationEnvelope,
  createSourceReference,
  parseAdviserContribution,
  parseCommandBrief,
  parseKnowledgeSnapshotManifest,
  parseMemoryRevision,
  parseModelRoute,
  parseProposedWorkspaceAction,
  parseReplicationEnvelope,
  parseSourceReference,
} from "./contracts.ts";
import {
  createCommandKnowledgeStatus,
  parseCommandKnowledgeStatus,
} from "./knowledgeStatus.ts";

const NOW = "2026-07-24T04:30:00.000Z";
const OFFSET_NOW = "2026-07-24T14:30:00+10:00";
const HASH_A = `sha256:${"a".repeat(64)}`;
const HASH_B = `sha256:${"b".repeat(64)}`;

function source(overrides = {}) {
  return createSourceReference({
    sourceId: "source-1",
    collection: "engineering-orders",
    documentId: "document-1",
    chunkId: "chunk-7",
    timestamp: NOW,
    snapshotId: "snapshot-1",
    quotedLocation: {
      quote: "Machinery state remains within operating limits.",
      location: "section 4, lines 12-18",
    },
    ...overrides,
  });
}

function action(actionType = "task", overrides = {}) {
  const variants = {
    task: {
      task: {
        title: "Review fuel figures",
        dueAt: NOW,
      },
    },
    "canvas-checklist-update": {
      update: {
        canvasId: "command-canvas",
        checklistId: "morning-checks",
        itemId: "fuel-review",
        completed: true,
      },
    },
    "scheduled-brief": {
      schedule: {
        briefId: "morning-command-brief",
        scheduledFor: NOW,
      },
    },
    "draft-message": {
      draft: {
        channelId: "command-team",
        body: "Draft only: fuel figures are ready for review.",
      },
    },
    "routing-action": {
      route: {
        adviser: "engineering",
        destination: "local-review-queue",
      },
    },
  };

  return createProposedWorkspaceAction({
    actionType,
    actionId: `action-${actionType}`,
    rationale: "Requires explicit command approval.",
    approvalState: "pending",
    ...variants[actionType],
    ...overrides,
  });
}

function contribution(overrides = {}) {
  return createAdviserContribution({
    adviser: "engineering",
    findings: ["Machinery state reviewed."],
    evidence: [source()],
    confidence: 0.85,
    limitations: ["Based on the latest replicated snapshot."],
    dissent: ["Logistics requests a second fuel calculation."],
    proposedActions: [action()],
    ...overrides,
  });
}

function memoryRevision(overrides = {}) {
  return createMemoryRevision({
    entityId: "hmas-supply",
    eventId: "memory-event-2",
    parentRevisionIds: ["memory-event-1"],
    nodeId: "command-node-1",
    timestamp: NOW,
    hashes: {
      content: HASH_A,
      revision: HASH_B,
    },
    tombstone: false,
    cursor: "cursor-0002",
    content: { status: "available", notes: ["local only"] },
    ...overrides,
  });
}

function fixtures() {
  const sourceReference = source();
  const proposedWorkspaceAction = action();
  const adviserContribution = contribution({
    evidence: [sourceReference],
    proposedActions: [proposedWorkspaceAction],
  });
  const commandBrief = createCommandBrief({
    contributions: [adviserContribution],
    consolidatedPriorities: ["Complete the engineering fuel review."],
    decisions: ["Retain the current operating posture."],
    sourceFreshness: {
      asOf: NOW,
      staleSourceIds: [],
    },
    generationAuditId: "audit-brief-1",
  });
  const modelRoute = createModelRoute({
    selectedEndpoint: "http://127.0.0.1:1234",
    selectedProvider: "lm-studio",
    selectedModel: "local-command-model",
    permittedTools: ["knowledge-retrieval"],
    fallbackChain: [
      {
        provider: "lm-studio",
        model: "local-command-fallback",
      },
    ],
    egressDecision: {
      allowed: false,
      rationale: "Local-only policy.",
    },
  });
  const knowledgeSnapshotManifest = createKnowledgeSnapshotManifest({
    snapshotId: "snapshot-1",
    createdAt: NOW,
    hashes: {
      manifest: HASH_A,
      content: HASH_B,
    },
    collections: [
      {
        collection: "engineering-orders",
        schemaVersion: "1",
        documentCount: 3,
        chunkCount: 12,
      },
    ],
    serviceRevision: "rag-service-revision-9",
    retrievalModelVersions: [
      {
        model: "local-embedding-model",
        version: "2",
      },
    ],
  });
  const revision = memoryRevision();
  const replicationEnvelope = createReplicationEnvelope({
    entityId: revision.entityId,
    eventId: "replication-event-3",
    parentRevisionIds: [revision.eventId],
    nodeId: revision.nodeId,
    timestamp: NOW,
    hashes: {
      payload: revision.hashes.revision,
      envelope: HASH_A,
    },
    tombstone: revision.tombstone,
    cursor: "cursor-0003",
    payload: revision,
  });

  return [
    [sourceReference, parseSourceReference],
    [adviserContribution, parseAdviserContribution],
    [commandBrief, parseCommandBrief],
    [proposedWorkspaceAction, parseProposedWorkspaceAction],
    [modelRoute, parseModelRoute],
    [knowledgeSnapshotManifest, parseKnowledgeSnapshotManifest],
    [revision, parseMemoryRevision],
    [replicationEnvelope, parseReplicationEnvelope],
  ];
}

test("every approved contract preserves all required fields through JSON", () => {
  for (const [artefact, parse] of fixtures()) {
    const persisted = JSON.parse(JSON.stringify(artefact));
    assert.deepEqual(parse(persisted), artefact, artefact.kind);
    assert.equal(JSON.stringify(persisted), JSON.stringify(artefact));
    assert.equal(Object.isFrozen(artefact), true, artefact.kind);
  }

  const [sourceReference, adviser, brief, proposedAction, route, manifest] =
    fixtures().map(([artefact]) => artefact);
  assert.deepEqual(Object.keys(sourceReference), [
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
  ]);
  assert.deepEqual(Object.keys(adviser), [
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
  ]);
  assert.deepEqual(Object.keys(brief), [
    "kind",
    "version",
    "classification",
    "contributions",
    "consolidatedPriorities",
    "decisions",
    "sourceFreshness",
    "generationAuditId",
  ]);
  assert.deepEqual(Object.keys(proposedAction), [
    "kind",
    "version",
    "classification",
    "actionType",
    "actionId",
    "rationale",
    "approvalState",
    "task",
  ]);
  assert.deepEqual(Object.keys(route), [
    "kind",
    "version",
    "classification",
    "selectedEndpoint",
    "selectedProvider",
    "selectedModel",
    "permittedTools",
    "fallbackChain",
    "egressDecision",
  ]);
  assert.deepEqual(Object.keys(manifest), [
    "kind",
    "version",
    "classification",
    "snapshotId",
    "createdAt",
    "hashes",
    "collections",
    "serviceRevision",
    "retrievalModelVersions",
  ]);
});

test("every creation helper defaults to OFFICIAL and leaf PUBLIC is preserved", () => {
  for (const [artefact] of fixtures()) {
    assert.equal(artefact.classification, "OFFICIAL", artefact.kind);
  }
  assert.equal(source({ classification: "PUBLIC" }).classification, "PUBLIC");
  assert.equal(
    action("task", { classification: "PUBLIC" }).classification,
    "PUBLIC",
  );
});

test("PUBLIC composites preserve PUBLIC and inherit nested OFFICIAL", () => {
  const publicSource = source({ classification: "PUBLIC" });
  const publicAction = action("task", { classification: "PUBLIC" });
  const publicContribution = contribution({
    evidence: [publicSource],
    proposedActions: [publicAction],
    classification: "PUBLIC",
  });
  assert.equal(publicContribution.classification, "PUBLIC");

  const elevatedContribution = contribution({
    evidence: [source()],
    proposedActions: [publicAction],
    classification: "PUBLIC",
  });
  assert.equal(elevatedContribution.classification, "OFFICIAL");

  const publicBrief = createCommandBrief({
    contributions: [publicContribution],
    consolidatedPriorities: ["Public priority"],
    decisions: ["Public decision"],
    sourceFreshness: { asOf: NOW, staleSourceIds: [] },
    generationAuditId: "audit-public",
    classification: "PUBLIC",
  });
  assert.equal(publicBrief.classification, "PUBLIC");

  const elevatedBrief = createCommandBrief({
    contributions: [elevatedContribution],
    consolidatedPriorities: ["Official priority"],
    decisions: ["Official decision"],
    sourceFreshness: { asOf: NOW, staleSourceIds: [] },
    generationAuditId: "audit-official",
    classification: "PUBLIC",
  });
  assert.equal(elevatedBrief.classification, "OFFICIAL");

  const publicRevision = memoryRevision({ classification: "PUBLIC" });
  const envelope = createReplicationEnvelope({
    entityId: publicRevision.entityId,
    eventId: "replication-public",
    parentRevisionIds: [publicRevision.eventId],
    nodeId: publicRevision.nodeId,
    timestamp: NOW,
    hashes: { payload: publicRevision.hashes.revision, envelope: HASH_A },
    tombstone: false,
    cursor: "cursor-public",
    payload: publicRevision,
    classification: "PUBLIC",
  });
  assert.equal(envelope.classification, "PUBLIC");

  const elevatedEnvelope = createReplicationEnvelope({
    ...envelope,
    eventId: "replication-elevated",
    payload: memoryRevision(),
    classification: "PUBLIC",
  });
  assert.equal(elevatedEnvelope.classification, "OFFICIAL");
});

test("workspace actions are a closed five-variant union", () => {
  for (const actionType of [
    "task",
    "canvas-checklist-update",
    "scheduled-brief",
    "draft-message",
    "routing-action",
  ]) {
    const proposed = action(actionType);
    assert.equal(
      parseProposedWorkspaceAction(proposed)?.actionType,
      actionType,
    );
  }

  const task = action();
  assert.equal(
    parseProposedWorkspaceAction({ ...task, actionType: "delete" }),
    null,
  );
  assert.equal(
    parseProposedWorkspaceAction({ ...task, approvalState: "executed" }),
    null,
  );
  const { task: _, ...missingVariant } = task;
  assert.equal(parseProposedWorkspaceAction(missingVariant), null);
});

test("parsers reject every missing top-level contract field", () => {
  for (const [artefact, parse] of fixtures()) {
    for (const key of Object.keys(artefact)) {
      const persisted = JSON.parse(JSON.stringify(artefact));
      delete persisted[key];
      assert.equal(parse(persisted), null, `${artefact.kind}.${key}`);
    }
  }
});

test("parsers reject malformed lineage, integrity, and tombstone metadata", () => {
  const revision = memoryRevision();
  assert.equal(
    parseMemoryRevision({ ...revision, parentRevisionIds: [""] }),
    null,
  );
  assert.equal(
    parseMemoryRevision({
      ...revision,
      hashes: { ...revision.hashes, content: "not-a-hash" },
    }),
    null,
  );
  assert.equal(parseMemoryRevision({ ...revision, tombstone: true }), null);

  const tombstone = memoryRevision({
    eventId: "memory-tombstone",
    tombstone: true,
    content: null,
  });
  assert.equal(tombstone.tombstone, true);

  const envelope = fixtures()[7][0];
  assert.equal(
    parseReplicationEnvelope({ ...envelope, entityId: "different-entity" }),
    null,
  );
  assert.equal(
    parseReplicationEnvelope({ ...envelope, tombstone: true }),
    null,
  );
  assert.equal(parseReplicationEnvelope({ ...envelope, cursor: "" }), null);
});

test("timestamps require valid RFC 3339 with an explicit offset", () => {
  assert.equal(source({ timestamp: OFFSET_NOW }).timestamp, OFFSET_NOW);

  for (const timestamp of [
    "1",
    "2026-07-24",
    "2026-07-24T04:30:00",
    "2026-02-30T04:30:00Z",
    "2026-07-24T25:00:00Z",
  ]) {
    const persisted = { ...source(), timestamp };
    assert.equal(parseSourceReference(persisted), null, timestamp);
  }
});

test("bounded JSON validation rejects excessive depth and cycles without throwing", () => {
  const base = memoryRevision();

  let tooDeep = "leaf";
  for (let index = 0; index < 200; index += 1) tooDeep = [tooDeep];
  assert.doesNotThrow(() => parseMemoryRevision({ ...base, content: tooDeep }));
  assert.equal(parseMemoryRevision({ ...base, content: tooDeep }), null);

  const cyclic = [];
  cyclic.push(cyclic);
  assert.doesNotThrow(() => parseMemoryRevision({ ...base, content: cyclic }));
  assert.equal(parseMemoryRevision({ ...base, content: cyclic }), null);

  let boundary = "leaf";
  for (let index = 0; index < 64; index += 1) boundary = [boundary];
  assert.notEqual(parseMemoryRevision({ ...base, content: boundary }), null);

  const overBoundary = [boundary];
  assert.equal(parseMemoryRevision({ ...base, content: overBoundary }), null);
});

test("__proto__ keys remain inert own JSON data at every nesting level", () => {
  const base = memoryRevision();
  const content = JSON.parse(
    '{"__proto__":{"polluted":true},"ok":true,"nested":{"__proto__":"inert","value":1}}',
  );

  const parsed = parseMemoryRevision({ ...base, content });
  assert.ok(parsed);
  assert.equal(Object.hasOwn(parsed.content, "__proto__"), true);
  assert.equal(Object.hasOwn(parsed.content.nested, "__proto__"), true);
  assert.equal(Object.getPrototypeOf(parsed.content), null);
  assert.equal(Object.getPrototypeOf(parsed.content.nested), null);
  assert.equal(parsed.content.__proto__.polluted, true);
  assert.equal(parsed.content.nested.__proto__, "inert");
  assert.equal(JSON.stringify(parsed.content), JSON.stringify(content));
  assert.equal({}.polluted, undefined);
});

test("command knowledge status preserves only bounded readiness metadata", () => {
  const status = createCommandKnowledgeStatus({
    observedAt: NOW,
    memory: {
      status: "ready",
      serverIdentity: "memory",
      nodeId: "node:command",
      homeNodeId: "node:home-command",
      revisionCount: 42,
      conflictCount: 2,
      replicationCursor: 41,
      homeReplicationCursor: 73,
      lastSuccessfulSync: NOW,
      freshness: "fresh",
      validation: "verified",
      toolAllowlist: ["get_entity", "recall_for_entity", "search_events"],
      error: null,
    },
    rag: {
      status: "ready",
      serverIdentity: "rag",
      activeSnapshotId: "f".repeat(64),
      signatureFingerprint: "e".repeat(64),
      snapshotTime: NOW,
      lastSuccessfulActivation: NOW,
      freshness: "fresh",
      validation: "verified",
      toolAllowlist: [
        "get_document",
        "get_snapshot_status",
        "list_collections",
        "search_knowledge_base",
      ],
      error: null,
    },
    appleInputs: [
      {
        source: "calendar",
        permission: "authorized",
        observedAt: NOW,
        recordCount: 0,
        truncated: false,
        error: null,
      },
      {
        source: "reminders",
        permission: "denied",
        observedAt: NOW,
        recordCount: 0,
        truncated: false,
        error: "permission_denied",
      },
      {
        source: "notes",
        permission: "authorized",
        observedAt: NOW,
        recordCount: 0,
        truncated: false,
        error: null,
      },
      {
        source: "files",
        permission: "authorized",
        observedAt: NOW,
        recordCount: 0,
        truncated: false,
        error: null,
      },
    ],
    degradedSections: ["apple-reminders", "memory-conflicts"],
  });

  assert.deepEqual(
    parseCommandKnowledgeStatus(JSON.parse(JSON.stringify(status))),
    status,
  );
  assert.deepEqual(Object.keys(status), [
    "kind",
    "version",
    "classification",
    "observedAt",
    "memory",
    "rag",
    "appleInputs",
    "degradedSections",
  ]);
  assert.equal(Object.isFrozen(status), true);
  assert.doesNotMatch(
    JSON.stringify(status),
    /content|credential|token|record fields/i,
  );
});

test("command knowledge status rejects asserted crypto state and unsafe metadata", () => {
  const base = createCommandKnowledgeStatus({
    observedAt: NOW,
    memory: {
      status: "unavailable",
      serverIdentity: null,
      nodeId: null,
      homeNodeId: null,
      revisionCount: 0,
      conflictCount: 0,
      replicationCursor: null,
      homeReplicationCursor: null,
      lastSuccessfulSync: null,
      freshness: "unknown",
      validation: "failed",
      toolAllowlist: [],
      error: "authentication_failed",
    },
    rag: {
      status: "unavailable",
      serverIdentity: null,
      activeSnapshotId: null,
      signatureFingerprint: null,
      snapshotTime: null,
      lastSuccessfulActivation: null,
      freshness: "unknown",
      validation: "failed",
      toolAllowlist: [],
      error: "snapshot_hash_mismatch",
    },
    appleInputs: ["calendar", "reminders", "notes", "files"].map((source) => ({
      source,
      permission: "authorized",
      observedAt: NOW,
      recordCount: 0,
      truncated: false,
      error: null,
    })),
    degradedSections: ["memory-readiness", "rag-readiness"],
  });

  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      rag: {
        ...base.rag,
        validation: "cryptographically-verified-by-renderer",
      },
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      memory: { ...base.memory, bearerToken: "secret" },
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      appleInputs: [
        {
          source: "files",
          permission: "authorized",
          observedAt: NOW,
          recordCount: 1,
          truncated: false,
          error: null,
          records: [{ fields: { text: "private" } }],
        },
      ],
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      memory: { ...base.memory, replicationCursor: -1 },
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      degradedSections: ["Bearer secret-token"],
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      memory: {
        ...base.memory,
        status: "ready",
        serverIdentity: "memory",
        nodeId: "node:command",
        freshness: "unknown",
        validation: "verified",
        error: null,
      },
    }),
    null,
  );
  assert.equal(
    parseCommandKnowledgeStatus({
      ...base,
      rag: {
        ...base.rag,
        status: "ready",
        serverIdentity: "rag",
        activeSnapshotId: "not-a-sha256-digest",
        signatureFingerprint: "e".repeat(64),
        snapshotTime: NOW,
        lastSuccessfulActivation: NOW,
        freshness: "fresh",
        validation: "verified",
        error: null,
      },
    }),
    null,
  );
});
