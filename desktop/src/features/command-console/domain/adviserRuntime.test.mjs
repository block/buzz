import assert from "node:assert/strict";
import test from "node:test";

import {
  APPROVED_ADVISERS,
  buildLmStudioNativeModelRoute,
  parseNativeAdviserContribution,
} from "./adviserRuntime.ts";

const NOW = "2026-07-24T04:30:00.000Z";

function source(overrides = {}) {
  return {
    kind: "source-reference",
    version: 1,
    classification: "OFFICIAL",
    sourceId: "source-1",
    collection: "navigation-orders",
    documentId: "document-1",
    chunkId: "chunk-7",
    timestamp: NOW,
    snapshotId: "snapshot-1",
    quotedLocation: {
      quote: "The source passage used by the finding.",
      location: "section 4, lines 12-18",
    },
    ...overrides,
  };
}

function contribution(overrides = {}) {
  return {
    kind: "adviser-contribution",
    version: 1,
    classification: "OFFICIAL",
    adviser: "Navigation",
    findings: ["The replicated source identifies a planning constraint."],
    evidence: [source()],
    confidence: 0.85,
    limitations: ["Source freshness remains subject to the snapshot time."],
    dissent: ["Operations recommends a second-source check."],
    proposedActions: [],
    ...overrides,
  };
}

function terminalMessages(value = contribution()) {
  return [JSON.stringify(value)];
}

test("accepts exactly one strict OFFICIAL contribution from the expected adviser", () => {
  const parsed = parseNativeAdviserContribution(
    terminalMessages(),
    "Navigation",
  );

  assert.ok(parsed);
  assert.equal(parsed.adviser, "Navigation");
  assert.deepEqual(parsed.limitations, [
    "Source freshness remains subject to the snapshot time.",
  ]);
  assert.deepEqual(parsed.dissent, [
    "Operations recommends a second-source check.",
  ]);
  assert.equal(Object.isFrozen(parsed), true);
});

test("pins the exact six Phase 1 adviser identities", () => {
  assert.deepEqual(APPROVED_ADVISERS, [
    "Chief of Staff",
    "Operations",
    "Navigation",
    "Daily Routine",
    "Reporting",
    "Plans",
  ]);

  for (const adviser of APPROVED_ADVISERS) {
    const parsed = parseNativeAdviserContribution(
      terminalMessages(contribution({ adviser })),
      adviser,
    );
    assert.equal(parsed?.adviser, adviser);
  }
});

test("rejects zero or multiple terminal messages", () => {
  assert.equal(parseNativeAdviserContribution([], "Navigation"), null);
  assert.equal(
    parseNativeAdviserContribution(
      [JSON.stringify(contribution()), JSON.stringify(contribution())],
      "Navigation",
    ),
    null,
  );
  assert.equal(
    parseNativeAdviserContribution([contribution()], "Navigation"),
    null,
  );
});

test("rejects Markdown fences, prose, and duplicate JSON values", () => {
  const json = JSON.stringify(contribution());
  for (const message of [
    `\`\`\`json\n${json}\n\`\`\``,
    `Here is the result: ${json}`,
    `${json}\nAdditional prose`,
    `${json}${json}`,
  ]) {
    assert.equal(
      parseNativeAdviserContribution([message], "Navigation"),
      null,
      message.slice(0, 40),
    );
  }
});

test("rejects duplicate object members before last-wins JSON parsing", () => {
  const valid = JSON.stringify(contribution());
  const topLevelAdjacent = valid.replace(
    '"adviser":"Navigation"',
    '"adviser":"Operations","adviser":"Navigation"',
  );
  const topLevelSeparated = valid
    .replace('"classification":"OFFICIAL"', '"classification":"PUBLIC"')
    .replace(
      '"adviser":"Navigation"',
      '"adviser":"Navigation","classification":"OFFICIAL"',
    );
  const escapedEquivalent = valid.replace(
    '"adviser":"Navigation"',
    '"\\u0061dviser":"Operations","adviser":"Navigation"',
  );
  const nestedSource = valid.replace(
    '"sourceId":"source-1"',
    '"sourceId":"other","sourceId":"source-1"',
  );
  const pendingAction = {
    kind: "proposed-workspace-action",
    version: 1,
    classification: "OFFICIAL",
    actionType: "task",
    actionId: "review-source",
    rationale: "Requires an explicit approval event.",
    approvalState: "pending",
    task: { title: "Review source", dueAt: NOW },
  };
  const nestedAction = JSON.stringify(
    contribution({ proposedActions: [pendingAction] }),
  ).replace(
    '"approvalState":"pending"',
    '"approvalState":"approved", "approvalState" : "pending"',
  );

  for (const message of [
    topLevelAdjacent,
    topLevelSeparated,
    escapedEquivalent,
    nestedSource,
    nestedAction,
  ]) {
    assert.equal(parseNativeAdviserContribution([message], "Navigation"), null);
  }
});

test("allows the same member name in separate sibling objects", () => {
  const parsed = parseNativeAdviserContribution(
    terminalMessages(
      contribution({
        evidence: [
          source(),
          source({
            sourceId: "source-2",
            documentId: "document-2",
            chunkId: "chunk-8",
          }),
        ],
      }),
    ),
    "Navigation",
  );

  assert.equal(parsed?.evidence.length, 2);
});

test("rejects adviser substitution and unapproved adviser names", () => {
  assert.equal(
    parseNativeAdviserContribution(terminalMessages(), "Operations"),
    null,
  );
  assert.equal(
    parseNativeAdviserContribution(
      terminalMessages(contribution({ adviser: "Navigation Adviser" })),
      "Navigation",
    ),
    null,
  );
});

test("rejects missing, extra, invalid classification, and unsupported fields", () => {
  const { confidence: _confidence, ...missing } = contribution();
  for (const candidate of [
    missing,
    { ...contribution(), unsupported: true },
    contribution({ classification: "PUBLIC" }),
    contribution({ classification: "SECRET" }),
    contribution({ evidence: [source({ classification: "PUBLIC" })] }),
  ]) {
    assert.equal(
      parseNativeAdviserContribution(terminalMessages(candidate), "Navigation"),
      null,
    );
  }
});

test("rejects invalid or non-finite confidence values", () => {
  for (const confidence of [-0.01, 1.01, "0.5", null]) {
    assert.equal(
      parseNativeAdviserContribution(
        terminalMessages(contribution({ confidence })),
        "Navigation",
      ),
      null,
    );
  }

  for (const raw of ["NaN", "Infinity", "-Infinity"]) {
    const message = JSON.stringify(contribution()).replace("0.85", raw);
    assert.equal(parseNativeAdviserContribution([message], "Navigation"), null);
  }
});

test("requires valid SourceReference evidence for factual findings", () => {
  assert.equal(
    parseNativeAdviserContribution(
      terminalMessages(contribution({ evidence: [] })),
      "Navigation",
    ),
    null,
  );
  assert.equal(
    parseNativeAdviserContribution(
      terminalMessages(
        contribution({
          evidence: [source({ timestamp: "not-a-timestamp" })],
        }),
      ),
      "Navigation",
    ),
    null,
  );

  const noFindings = parseNativeAdviserContribution(
    terminalMessages(contribution({ findings: [], evidence: [] })),
    "Navigation",
  );
  assert.ok(noFindings);
});

test("adviser output cannot self-approve a proposed workspace action", () => {
  const proposedAction = {
    kind: "proposed-workspace-action",
    version: 1,
    classification: "OFFICIAL",
    actionType: "task",
    actionId: "review-source",
    rationale: "Requires an explicit approval event.",
    approvalState: "approved",
    task: {
      title: "Review the cited source",
      dueAt: NOW,
    },
  };

  assert.equal(
    parseNativeAdviserContribution(
      terminalMessages(contribution({ proposedActions: [proposedAction] })),
      "Navigation",
    ),
    null,
  );
});

test("rejects dangerous keys and control characters at any nesting level", () => {
  const dangerous = JSON.stringify(contribution()).replace(
    '"location":"section 4, lines 12-18"',
    '"location":"section 4, lines 12-18","__proto__":{"polluted":true}',
  );
  const control = JSON.stringify(
    contribution({ dissent: ["Unsafe\u0000control"] }),
  );

  assert.equal(parseNativeAdviserContribution([dangerous], "Navigation"), null);
  assert.equal(parseNativeAdviserContribution([control], "Navigation"), null);
  assert.equal({}.polluted, undefined);
});

test("rejects oversized strings, arrays, messages, and excessive nesting", () => {
  const tooMany = Array.from({ length: 65 }, (_, index) => `item-${index}`);
  const deeplyNested = JSON.stringify(contribution()).replace(
    '"proposedActions":[]',
    `"proposedActions":[${JSON.stringify({
      kind: "proposed-workspace-action",
      version: 1,
      classification: "OFFICIAL",
      actionType: "task",
      actionId: "a",
      rationale: "r",
      approvalState: "pending",
      task: { title: "t", dueAt: NOW },
    }).replace(
      '"title":"t"',
      `"title":${"[".repeat(20)}"t"${"]".repeat(20)}`,
    )}]`,
  );

  for (const message of [
    JSON.stringify(contribution({ limitations: ["x".repeat(16_385)] })),
    JSON.stringify(contribution({ limitations: tooMany })),
    JSON.stringify(contribution({ findings: ["x".repeat(300_000)] })),
    deeplyNested,
  ]) {
    assert.equal(parseNativeAdviserContribution([message], "Navigation"), null);
  }
});

test("builds an OFFICIAL local-only LM Studio route from Rust policy evidence", () => {
  const route = buildLmStudioNativeModelRoute({
    endpoint: "http://127.0.0.1:1234",
    model: "qwen/qwen3.6-27b",
    permittedTools: ["memory.search", "rag.retrieve"],
    rustEgressDecision: {
      allowed: true,
      rationale: "literal loopback endpoint and allowlisted native MCP tools",
    },
  });

  assert.deepEqual(route, {
    kind: "model-route",
    version: 1,
    classification: "OFFICIAL",
    selectedEndpoint: "http://127.0.0.1:1234",
    selectedProvider: "lmstudio-native",
    selectedModel: "qwen/qwen3.6-27b",
    permittedTools: ["memory.search", "rag.retrieve"],
    fallbackChain: [],
    egressDecision: {
      allowed: true,
      rationale:
        "Rust enforcement authority: literal loopback endpoint and allowlisted native MCP tools",
    },
  });
});

test("accepts and canonicalizes all four Rust-authorized root endpoint spellings", () => {
  for (const [endpoint, canonical] of [
    ["http://127.0.0.1:1234", "http://127.0.0.1:1234"],
    ["http://127.0.0.1:1234/", "http://127.0.0.1:1234"],
    ["http://[::1]:1234", "http://[::1]:1234"],
    ["http://[::1]:1234/", "http://[::1]:1234"],
  ]) {
    const route = buildLmStudioNativeModelRoute({
      endpoint,
      model: "local-model",
      permittedTools: [],
      rustEgressDecision: { allowed: true, rationale: "allowed" },
    });
    assert.equal(route.selectedEndpoint, canonical);
  }
});

test("matches Rust URL normalization for explicit HTTP ports", () => {
  for (const endpoint of [
    "http://127.0.0.1:80",
    "http://127.0.0.1:80/",
    "http://[::1]:80",
    "http://[::1]:80/",
  ]) {
    assert.throws(
      () =>
        buildLmStudioNativeModelRoute({
          endpoint,
          model: "local-model",
          permittedTools: [],
          rustEgressDecision: { allowed: false, rationale: "denied" },
        }),
      TypeError,
    );
  }

  for (const port of [1, 443, 1234, 65_535]) {
    for (const host of ["127.0.0.1", "[::1]"]) {
      const endpoint = `http://${host}:${port}`;
      const route = buildLmStudioNativeModelRoute({
        endpoint,
        model: "local-model",
        permittedTools: [],
        rustEgressDecision: { allowed: true, rationale: "allowed" },
      });
      assert.equal(route.selectedEndpoint, endpoint);
    }
  }
});

test("the display route rejects non-literal-local endpoints and invalid policy input", () => {
  for (const endpoint of [
    "http://localhost:1234",
    "http://127.0.0.2:1234",
    "http://127.1:1234",
    "http://2130706433:1234",
    "http://192.168.1.10:1234",
    "http://8.8.8.8:1234",
    "https://127.0.0.1:1234",
    "http://127.0.0.1",
    "http://127.0.0.1:1234/path",
    "http://127.0.0.1:1234//",
    "http://127.0.0.1:1234?query=1",
    "http://127.0.0.1:1234/#fragment",
    "http://user@127.0.0.1:1234",
    "http://0x7f000001:1234",
    "http://0177.0.0.1:1234",
    "http://[::ffff:127.0.0.1]:1234",
  ]) {
    assert.throws(
      () =>
        buildLmStudioNativeModelRoute({
          endpoint,
          model: "local-model",
          permittedTools: [],
          rustEgressDecision: { allowed: false, rationale: "denied" },
        }),
      TypeError,
    );
  }

  assert.throws(
    () =>
      buildLmStudioNativeModelRoute({
        endpoint: "http://[::1]:1234",
        model: "local-model",
        permittedTools: [],
        rustEgressDecision: { allowed: true, rationale: "" },
      }),
    TypeError,
  );

  for (const permittedTools of [
    ["memory.search", "memory.search"],
    Array.from({ length: 65 }, (_, index) => `memory.tool-${index}`),
  ]) {
    assert.throws(
      () =>
        buildLmStudioNativeModelRoute({
          endpoint: "http://127.0.0.1:1234",
          model: "local-model",
          permittedTools,
          rustEgressDecision: { allowed: true, rationale: "allowed" },
        }),
      TypeError,
    );
  }
});
