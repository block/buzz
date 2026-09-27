import assert from "node:assert/strict";
import test from "node:test";

import {
  emptyRunbook,
  parseCommunityRunbookPayload,
  parseSiteRunbook,
  parseSiteRunbooksBlob,
  shapeRunbookForCommunity,
  shapeRunbookInject,
  summarizeProcedureSteps,
} from "./serialize.ts";
import {
  acceptProcedure,
  proposeProcedure,
  setProcedurePersisted,
  upsertAgentProcedure,
  setAgentBrief,
} from "./mutations.ts";
import {
  encodeRunbookRef,
  parseRunbookRef,
  pinRunbookRef,
  sidRunbookRef,
} from "./keys.ts";

test("encode/parse runbook refs", () => {
  assert.equal(encodeRunbookRef(pinRunbookRef("abc")), "pin:abc");
  assert.equal(encodeRunbookRef(sidRunbookRef("s1")), "sid:s1");
  assert.deepEqual(parseRunbookRef("pin:abc"), { kind: "pin", pinId: "abc" });
  assert.deepEqual(parseRunbookRef("sid:s1"), { kind: "sid", sid: "s1" });
  assert.equal(parseRunbookRef("nope"), null);
});

test("parseSiteRunbook drops bad procedures and keeps fields", () => {
  const parsed = parseSiteRunbook({
    agentBrief: "Log in first",
    updatedAt: 10,
    procedures: [
      {
        id: "p1",
        title: "Search",
        steps: "1. Click search",
        status: "active",
        createdAt: 1,
        updatedAt: 2,
      },
      { id: "bad", title: "", steps: "x", status: "active" },
      { id: "p1", title: "dup", steps: "x", status: "pending" },
    ],
  });
  assert.equal(parsed?.agentBrief, "Log in first");
  assert.equal(parsed?.procedures.length, 1);
  assert.equal(parsed?.procedures[0].id, "p1");
});

test("parseSiteRunbooksBlob rejects wrong version", () => {
  assert.equal(parseSiteRunbooksBlob({ version: 2, runbooks: {} }), null);
});

test("summarizeProcedureSteps collapses whitespace and truncates", () => {
  assert.equal(summarizeProcedureSteps("a\n\nb"), "a b");
  const long = "x".repeat(200);
  const summary = summarizeProcedureSteps(long, 20);
  assert.ok(summary.length <= 20);
  assert.ok(summary.endsWith("…"));
});

test("shapeRunbookInject includes only active procedures", () => {
  let runbook = emptyRunbook(100);
  runbook = setAgentBrief(runbook, "Use the sidebar");
  runbook = {
    ...runbook,
    procedures: [
      {
        id: "pend",
        title: "Pending tip",
        steps: "Do not show yet",
        status: "pending",
        createdAt: 1,
        updatedAt: 1,
      },
      {
        id: "a1",
        title: "Active tip",
        steps: "Click Save\nConfirm",
        status: "active",
        createdAt: 1,
        updatedAt: 1,
      },
      {
        id: "z1",
        title: "Old",
        steps: "gone",
        status: "archived",
        createdAt: 1,
        updatedAt: 1,
      },
    ],
  };
  const inject = shapeRunbookInject(runbook);
  assert.equal(inject.agentBrief, "Use the sidebar");
  assert.equal(inject.procedures.length, 1);
  assert.equal(inject.procedures[0].id, "a1");
  assert.equal(inject.procedures[0].summary, "Click Save Confirm");
  assert.ok(Array.isArray(inject.driveProtocol));
  assert.ok(inject.driveProtocol.some((line) => line.includes("surfaceId")));
  assert.ok(
    inject.driveProtocol.some((line) => line.includes("host screen")),
    "protocol mentions host-owned screens",
  );
  assert.ok(
    inject.driveProtocol.some((line) => line.includes("browser_switch_tab")),
    "protocol mentions tab switch",
  );
  assert.ok(
    inject.driveProtocol.some(
      (line) =>
        line.includes("userHasControl") &&
        line.includes("snapshots") &&
        line.includes("Do not Drive until Release"),
    ),
    "protocol tells agents how to follow while Taken",
  );
  assert.ok(
    !inject.driveProtocol.some((line) =>
      line.includes("Do not call browser_snapshot(screenshot=true) every step"),
    ),
  );
});

test("agent upsert auto-activates", () => {
  const { runbook, procedure } = upsertAgentProcedure(null, {
    title: "How to filter",
    steps: "Open Filters",
    sourceAgent: "agent1",
  });
  assert.equal(procedure.status, "active");
  assert.equal(procedure.persisted, false);
  assert.ok(procedure.acceptedAt);
  assert.equal(runbook.procedures[0].status, "active");
});

test("agent upsert replaces non-persisted same title", () => {
  let { runbook } = upsertAgentProcedure(null, {
    title: "Login",
    steps: "v1",
  });
  const next = upsertAgentProcedure(runbook, {
    title: "login",
    steps: "v2",
  });
  assert.equal(next.runbook.procedures.length, 1);
  assert.equal(next.procedure.steps, "v2");
});

test("agent upsert rejects persisted procedure", () => {
  let { runbook, procedure } = upsertAgentProcedure(null, {
    title: "Locked",
    steps: "secret",
  });
  runbook = setProcedurePersisted(runbook, procedure.id, true);
  assert.throws(() => {
    upsertAgentProcedure(runbook, { title: "Locked", steps: "hack" });
  }, /persisted/i);
});

test("propose alias auto-activates (compat)", () => {
  const { procedure } = proposeProcedure(null, {
    title: "How to filter",
    steps: "Open Filters",
    sourceAgent: "agent1",
  });
  assert.equal(procedure.status, "active");
});

test("community payload omits pending and round-trips active", () => {
  let runbook = setAgentBrief(null, "Brief");
  runbook = {
    ...runbook,
    procedures: [
      {
        id: "pend",
        title: "Pending",
        steps: "no",
        status: "pending",
        createdAt: 1,
        updatedAt: 1,
      },
      {
        id: "ok",
        title: "Login",
        steps: "Enter email",
        status: "active",
        createdAt: 1,
        updatedAt: 2,
        acceptedAt: 2,
      },
    ],
  };
  const payload = shapeRunbookForCommunity(runbook);
  assert.ok(payload);
  assert.equal(payload.procedures.length, 1);
  assert.equal(payload.procedures[0].id, "ok");
  const parsed = parseCommunityRunbookPayload(payload);
  assert.equal(parsed?.procedures[0].status, "active");
  assert.ok(!parsed?.procedures.some((p) => p.id === "pend"));
});
