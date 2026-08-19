import assert from "node:assert/strict";
import test from "node:test";

import {
  canAuthorWebhookWorkflow,
  getChannelRoleForPubkey,
  workflowUsesCallWebhook,
} from "./workflowAuthz.ts";

test("workflowUsesCallWebhook detects webhook actions in valid workflow YAML", () => {
  assert.equal(
    workflowUsesCallWebhook(`
name: Send to n8n
trigger:
  on: message_posted
steps:
  - id: call
    action: call_webhook
    url: https://example.com/hook
`),
    true,
  );
});

test("workflowUsesCallWebhook ignores workflows without webhook actions", () => {
  assert.equal(
    workflowUsesCallWebhook(`
name: React
trigger:
  on: reaction_added
steps:
  - id: react
    action: add_reaction
    emoji: eyes
`),
    false,
  );
});

test("workflowUsesCallWebhook lets malformed YAML fall through to relay validation", () => {
  assert.equal(workflowUsesCallWebhook("{ name: broken"), false);
});

test("getChannelRoleForPubkey is case-insensitive", () => {
  assert.equal(
    getChannelRoleForPubkey(
      [
        {
          pubkey: "ABCDEF",
          role: "owner",
          isAgent: false,
          joinedAt: "",
          displayName: null,
        },
      ],
      "abcdef",
    ),
    "owner",
  );
});

test("canAuthorWebhookWorkflow allows only owner and admin", () => {
  assert.equal(canAuthorWebhookWorkflow("owner"), true);
  assert.equal(canAuthorWebhookWorkflow("admin"), true);
  assert.equal(canAuthorWebhookWorkflow("member"), false);
  assert.equal(canAuthorWebhookWorkflow("bot"), false);
  assert.equal(canAuthorWebhookWorkflow(null), false);
});
