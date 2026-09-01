import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";
import {
  clearRetainedAgentationSubmission,
  readRetainedAgentationSubmission,
  retainAgentationSubmission,
} from "./agentationSubmissionStore.ts";

const dom = new JSDOM("<!doctype html>", { url: "http://localhost" });
Object.assign(globalThis, { localStorage: dom.window.localStorage });
const event = {
  id: "stable-event",
  pubkey: "alice",
  created_at: 1,
  kind: 9,
  tags: [["h", "channel"]],
  content: "feedback",
  sig: "signature",
};

beforeEach(() => localStorage.clear());

test("an ambiguous submission retains the exact signed event for retry", () => {
  retainAgentationSubmission("scope", {
    fingerprint: "batch",
    submissionId: "submission",
    annotations: [{ id: "annotation-a", comment: "original" }],
    channelId: "channel",
    agentPubkey: "agent",
    event,
  });

  assert.equal(
    readRetainedAgentationSubmission("scope")?.annotations[0]?.comment,
    "original",
  );
  assert.equal(readRetainedAgentationSubmission("scope")?.event.id, event.id);
  assert.equal(readRetainedAgentationSubmission("scope")?.event.sig, event.sig);
  clearRetainedAgentationSubmission("scope");
  assert.equal(readRetainedAgentationSubmission("scope"), null);
});
