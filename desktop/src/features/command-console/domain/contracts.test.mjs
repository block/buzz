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

const NOW = "2026-07-24T04:30:00.000Z";

function source(overrides = {}) {
  return createSourceReference({
    id: "source-1",
    title: "Engineering order",
    locator: "file:///orders/engineering.md",
    capturedAt: NOW,
    ...overrides,
  });
}

function contribution(overrides = {}) {
  return createAdviserContribution({
    id: "contribution-1",
    adviser: "Engineering",
    summary: "Machinery state reviewed.",
    sources: [source()],
    producedAt: NOW,
    ...overrides,
  });
}

function fixtures() {
  const sourceReference = source();
  const adviserContribution = contribution({ sources: [sourceReference] });
  const commandBrief = createCommandBrief({
    id: "brief-1",
    title: "Morning command brief",
    summary: "No critical changes.",
    contributions: [adviserContribution],
    createdAt: NOW,
  });
  const proposedWorkspaceAction = createProposedWorkspaceAction({
    id: "action-1",
    operation: "update",
    target: "briefs/morning.md",
    rationale: "Publish the approved command brief.",
    proposedAt: NOW,
  });
  const modelRoute = createModelRoute({
    id: "route-1",
    adviser: "Engineering",
    provider: "lm-studio",
    model: "local-model",
    rationale: "Local route selected by policy.",
    selectedAt: NOW,
  });
  const knowledgeSnapshotManifest = createKnowledgeSnapshotManifest({
    id: "snapshot-1",
    createdAt: NOW,
    checksum: "sha256:abc123",
    sources: [sourceReference],
  });
  const memoryRevision = createMemoryRevision({
    id: "memory-1",
    entityId: "hmas-supply",
    revision: 1,
    revisedAt: NOW,
    content: { status: "available", notes: ["local only"] },
  });
  const replicationEnvelope = createReplicationEnvelope({
    id: "replication-1",
    sequence: 0,
    createdAt: NOW,
    payload: memoryRevision,
  });

  return [
    [sourceReference, parseSourceReference],
    [adviserContribution, parseAdviserContribution],
    [commandBrief, parseCommandBrief],
    [proposedWorkspaceAction, parseProposedWorkspaceAction],
    [modelRoute, parseModelRoute],
    [knowledgeSnapshotManifest, parseKnowledgeSnapshotManifest],
    [memoryRevision, parseMemoryRevision],
    [replicationEnvelope, parseReplicationEnvelope],
  ];
}

test("all command-domain creation helpers default to OFFICIAL", () => {
  for (const [artefact] of fixtures()) {
    assert.equal(artefact.classification, "OFFICIAL", artefact.kind);
    assert.equal(artefact.version, 1, artefact.kind);
  }
});

test("command-domain artefacts are deeply immutable and JSON serialisable", () => {
  for (const [artefact] of fixtures()) {
    assert.equal(Object.isFrozen(artefact), true, artefact.kind);
    assert.deepEqual(JSON.parse(JSON.stringify(artefact)), artefact);
  }

  const brief = fixtures()[2][0];
  assert.equal(Object.isFrozen(brief.contributions), true);
  assert.equal(Object.isFrozen(brief.contributions[0]), true);
  assert.equal(Object.isFrozen(brief.contributions[0].sources), true);

  const revision = fixtures()[6][0];
  assert.equal(Object.isFrozen(revision.content), true);
  assert.equal(Object.isFrozen(revision.content.notes), true);
});

test("composite helpers preserve the highest nested classification", () => {
  const protectedSource = source({ classification: "PROTECTED" });
  const protectedContribution = contribution({
    classification: "OFFICIAL",
    sources: [protectedSource],
  });
  assert.equal(protectedContribution.classification, "PROTECTED");

  const protectedBrief = createCommandBrief({
    id: "brief-protected",
    title: "Protected brief",
    summary: "Contains protected material.",
    contributions: [protectedContribution],
    createdAt: NOW,
    classification: "OFFICIAL",
  });
  assert.equal(protectedBrief.classification, "PROTECTED");

  const protectedManifest = createKnowledgeSnapshotManifest({
    id: "snapshot-protected",
    createdAt: NOW,
    checksum: "sha256:def456",
    sources: [protectedSource],
    classification: "OFFICIAL",
  });
  assert.equal(protectedManifest.classification, "PROTECTED");

  const envelope = createReplicationEnvelope({
    id: "replication-protected",
    sequence: 1,
    createdAt: NOW,
    payload: protectedManifest,
    classification: "OFFICIAL",
  });
  assert.equal(envelope.classification, "PROTECTED");
});

test("narrow parsers accept JSON round trips and return frozen copies", () => {
  for (const [artefact, parse] of fixtures()) {
    const persisted = JSON.parse(JSON.stringify(artefact));
    const parsed = parse(persisted);

    assert.deepEqual(parsed, artefact, artefact.kind);
    assert.notEqual(parsed, persisted, artefact.kind);
    assert.equal(Object.isFrozen(parsed), true, artefact.kind);
  }
});

test("narrow parsers reject malformed, extra, or downgraded persisted data", () => {
  for (const [artefact, parse] of fixtures()) {
    assert.equal(parse({ ...artefact, unexpected: true }), null, artefact.kind);
    assert.equal(
      parse({ ...artefact, classification: "UNCLASSIFIED" }),
      null,
      artefact.kind,
    );
  }

  const protectedSource = source({ classification: "PROTECTED" });
  const downgradedContribution = {
    ...contribution({ sources: [protectedSource] }),
    classification: "OFFICIAL",
  };
  assert.equal(parseAdviserContribution(downgradedContribution), null);

  const revision = fixtures()[6][0];
  assert.equal(parseMemoryRevision({ ...revision, revision: -1 }), null);
  assert.equal(
    parseMemoryRevision({ ...revision, content: { invalid: undefined } }),
    null,
  );
});
