import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";
import { agentationPathname } from "./agentationPendingStore.ts";
import {
  discardAgentationPendingBundle,
  readAgentationPendingBundle,
} from "./agentationOffboarding.ts";
import { retainAgentationSubmission } from "./agentationSubmissionStore.ts";

const dom = new JSDOM("<!doctype html>", { url: "http://localhost/one" });
Object.assign(globalThis, {
  Event: dom.window.Event,
  localStorage: dom.window.localStorage,
  window: dom.window,
});
const event = {
  id: "stable-event",
  pubkey: "alice",
  created_at: 1,
  kind: 9,
  tags: [["h", "channel"]],
  content: "sensitive feedback",
  sig: "signature",
};

beforeEach(() => localStorage.clear());

test("export bundle includes retained signed payload and discard clears both stores", () => {
  const scope = "scope";
  localStorage.setItem(
    `feedback-annotations-${agentationPathname(scope)}`,
    JSON.stringify([{ id: "annotation-a" }]),
  );
  retainAgentationSubmission(scope, {
    fingerprint: "batch",
    submissionId: "submission",
    annotations: [{ id: "annotation-a", comment: "original" }],
    channelId: "channel",
    agentPubkey: "agent",
    event,
  });
  const bundle = readAgentationPendingBundle(scope);
  assert.deepEqual(bundle.annotations, [{ id: "annotation-a" }]);
  assert.equal(bundle.retainedSubmission?.event.content, "sensitive feedback");
  discardAgentationPendingBundle(scope);
  assert.deepEqual(readAgentationPendingBundle(scope), {
    annotations: [],
    retainedSubmission: null,
  });
});
