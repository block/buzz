import assert from "node:assert/strict";
import { test } from "node:test";
import { finalizeEvent, generateSecretKey, verifyEvent } from "nostr-tools";
import { WorkflowTriggerOperations } from "./triggerOperations.ts";

const scope = {
  expectedRelayUrl: "wss://a.example",
  expectedSignerPubkey: "a".repeat(64),
};
const signed = () =>
  finalizeEvent(
    {
      kind: 46020,
      created_at: 42,
      content: "",
      tags: [
        ["d", "workflow"],
        ["request-id", crypto.randomUUID()],
      ],
    },
    generateSecretKey(),
  );

test("production owner retains exact signed payload after committed response loss; concurrent callers join; distinct run is explicit", async () => {
  const posts = [];
  const runs = new Map();
  let prepared = 0;
  let loseResponse = true;
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const owner = new WorkflowTriggerOperations({
    prepareWorkflowTrigger: async () => {
      prepared++;
      await gate;
      return signed();
    },
    triggerWorkflow: async (_id, event) => {
      assert.ok(verifyEvent(event));
      posts.push(JSON.stringify(event));
      if (!runs.has(event.id)) runs.set(event.id, `run-${runs.size + 1}`);
      if (loseResponse) {
        loseResponse = false;
        throw new Error("response lost after commit");
      }
      return {
        runId: runs.get(event.id),
        workflowId: "workflow",
        status: "pending",
      };
    },
  });
  const key = owner.key("workflow", scope);
  const first = owner.run("workflow", scope);
  assert.equal(owner.state(key).status, "pending");
  assert.equal(owner.run("workflow", scope), first);
  release();
  await assert.rejects(first, /response lost/);
  assert.equal(owner.state(key).status, "error");
  assert.equal(owner.state(key).failurePhase, "submit");
  const result = await owner.run("workflow", scope);
  assert.equal(result.runId, "run-1");
  assert.equal(prepared, 1);
  assert.equal(posts[0], posts[1]);
  assert.equal(runs.size, 1);
  assert.equal(owner.state(key).status, "success");
  const distinct = await owner.run("workflow", scope);
  assert.equal(distinct.runId, "run-2");
  assert.equal(runs.size, 2);
});

test("scope-keyed retry survives A to B to A; captured caller scope cannot be mutated during await", async () => {
  const seen = [];
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const owner = new WorkflowTriggerOperations({
    prepareWorkflowTrigger: async () => {
      await gate;
      return signed();
    },
    triggerWorkflow: async (_id, event, captured) => {
      seen.push([event.id, { ...captured }]);
      throw new Error("unknown");
    },
  });
  const mutable = { ...scope };
  const first = owner.run("workflow", mutable);
  mutable.expectedRelayUrl = "wss://b.example";
  assert.equal(owner.state(owner.key("workflow", mutable)).status, "idle");
  assert.equal(
    owner.state(
      owner.key("workflow", { ...scope, expectedSignerPubkey: "b".repeat(64) }),
    ).status,
    "idle",
  );
  release();
  await assert.rejects(first);
  await assert.rejects(owner.run("workflow", scope));
  assert.deepEqual(seen[0], seen[1]);
  assert.deepEqual(seen[0][1], scope);
  await assert.rejects(owner.run("workflow", scope, true));
  assert.notEqual(
    seen[2][0],
    seen[0][0],
    "explicit abandon creates a distinct signed trigger",
  );
});

test("preflight rejection retries preparation without inventing a submitted event", async () => {
  let tries = 0;
  const owner = new WorkflowTriggerOperations({
    prepareWorkflowTrigger: async () => {
      tries++;
      throw new Error("stale revision");
    },
    triggerWorkflow: async () =>
      assert.fail("preflight failure must not publish"),
  });
  await assert.rejects(owner.run("workflow", scope), /stale revision/);
  assert.equal(
    owner.state(owner.key("workflow", scope)).failurePhase,
    "prepare",
  );
  await assert.rejects(owner.run("workflow", scope), /stale revision/);
  assert.equal(tries, 2);
});
