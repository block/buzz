import assert from "node:assert/strict";
import test from "node:test";

import {
  getWorkflowActionTiles,
  getWorkflowCardLabel,
  getWorkflowDisplayStatus,
  getWorkflowPrimaryAction,
  getWorkflowPrimaryActionEmoji,
  getWorkflowStepCount,
  getWorkflowTriggerEmoji,
  getWorkflowTriggerSummary,
  getWorkflowTriggerType,
  withWorkflowEnabled,
} from "./workflowDefinition.ts";

test("reads only direct trigger and first-action types for card icons", () => {
  assert.equal(
    getWorkflowTriggerType({ trigger: { on: "message_posted" } }),
    "message_posted",
  );
  assert.equal(
    getWorkflowPrimaryAction({
      steps: [{ action: "send_message" }, { action: "delay" }],
    }),
    "send_message",
  );
  assert.equal(getWorkflowTriggerType({ trigger: { on: "" } }), null);
  assert.equal(getWorkflowPrimaryAction({ steps: [null] }), null);
});

test("builds a plain-language workflow card label", () => {
  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "message_posted" },
      steps: [{ action: "send_message", text: "Deploying now" }],
    }),
    "When a message is posted, send “Deploying now”",
  );

  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "reaction_added", emoji: "🔥" },
      steps: [
        { action: "delay", duration: "5m" },
        { action: "add_reaction", emoji: "✅" },
      ],
    }),
    "When someone reacts with 🔥, wait 5 minutes, then 1 more step",
  );

  assert.equal(
    getWorkflowCardLabel(
      {
        trigger: {
          on: "reaction_added",
          filter: 'trigger_emoji == "🫠"',
        },
        steps: [{ action: "add_reaction", emoji: "👍" }],
      },
      { triggerReaction: "🫠" },
    ),
    "When someone reacts with 🫠, add a 👍 reaction",
  );

  assert.equal(
    getWorkflowCardLabel({
      trigger: {
        on: "reaction_added",
        filter: 'trigger_emoji == "🫠"',
      },
      steps: [{ action: "add_reaction", emoji: "👍" }],
    }),
    "When someone reacts with 🫠, add a 👍 reaction",
  );

  assert.equal(
    getWorkflowCardLabel(
      {
        trigger: { on: "message_posted" },
        steps: [{ action: "send_message", text: "Deploying now" }],
      },
      { actionChannelLabel: "releases" },
    ),
    "When a message is posted, send “Deploying now” in #releases",
  );

  assert.equal(
    getWorkflowCardLabel(
      {
        trigger: {
          on: "message_posted",
          filter: `trigger_author == "${"a".repeat(64)}"`,
        },
        steps: [{ action: "send_message", text: "Deploying now" }],
      },
      { triggerDescription: "Message posted by Carl" },
    ),
    "When a message is posted by Carl, send “Deploying now”",
  );

  assert.equal(
    getWorkflowCardLabel(
      {
        trigger: {
          on: "message_posted",
          filter: 'str_contains(trigger_text, "deploy")',
        },
        steps: [{ action: "call_webhook" }],
      },
      { triggerDescription: "Message contains “deploy”" },
    ),
    "When a message contains “deploy”, call a webhook",
  );

  assert.equal(
    getWorkflowCardLabel({
      trigger: {
        on: "message_posted",
        filter: 'str_contains(trigger_text, "deploy")',
      },
      steps: [{ action: "call_webhook" }],
    }),
    "When a message contains “deploy”, call a webhook",
  );

  assert.equal(
    getWorkflowCardLabel({
      trigger: {
        on: "reaction_added",
        filter: `trigger_message_id == "${"b".repeat(64)}"`,
      },
      steps: [{ action: "add_reaction", emoji: "👍" }],
    }),
    "When a reaction is added, add a 👍 reaction",
  );

  assert.equal(
    getWorkflowCardLabel(
      {
        trigger: {
          on: "message_posted",
          filter: `trigger_text == "FUCK" && trigger_author == "${"a".repeat(64)}"`,
        },
        steps: [{ action: "send_message", text: "{{trigger.text}} yourself" }],
      },
      {
        triggerDescription: "Message “FUCK” is posted by Carl",
      },
    ),
    "When “FUCK” is posted by Carl, send “{{trigger.text}} yourself”",
  );
});

test("builds a concise semantic trigger label for workflow cards", () => {
  assert.equal(
    getWorkflowTriggerSummary({
      trigger: {
        on: "message_posted",
        filter: 'str_contains(trigger_text, "deploy")',
      },
    }),
    "Message contains “deploy”",
  );
  assert.equal(
    getWorkflowTriggerSummary({
      trigger: {
        on: "reaction_added",
        filter: `trigger_emoji == "🔥" && trigger_author == "${"a".repeat(64)}" && trigger_message_id == "${"b".repeat(64)}"`,
      },
    }),
    "🔥 reaction added",
  );
  assert.equal(
    getWorkflowTriggerSummary({
      trigger: { on: "schedule", interval: "15m" },
    }),
    "Schedule",
  );
});

test("counts configured steps for card stack presentation", () => {
  assert.equal(getWorkflowStepCount({}), 0);
  assert.equal(
    getWorkflowStepCount({
      steps: [{ action: "send_message" }, { action: "add_reaction" }],
    }),
    2,
  );
});

test("returns configured reaction emoji for rich card rendering", () => {
  assert.equal(
    getWorkflowTriggerEmoji({
      trigger: {
        on: "reaction_added",
        filter: 'trigger_emoji == "👀"',
      },
    }),
    "👀",
  );
  assert.equal(
    getWorkflowTriggerEmoji({
      trigger: { on: "reaction_added", emoji: ":blob-wave:" },
    }),
    ":blob-wave:",
  );
  assert.equal(
    getWorkflowTriggerEmoji({
      trigger: {
        on: "reaction_added",
        filter: 'trigger_emoji != "👀"',
      },
    }),
    null,
  );
  assert.equal(
    getWorkflowTriggerEmoji({
      trigger: {
        on: "message_posted",
        filter: 'trigger_emoji == "👀"',
      },
    }),
    null,
  );

  assert.equal(
    getWorkflowPrimaryActionEmoji({
      steps: [{ action: "add_reaction", emoji: ":blob-wave:" }],
    }),
    ":blob-wave:",
  );
  assert.equal(
    getWorkflowPrimaryActionEmoji({
      steps: [{ action: "send_message", emoji: ":blob-wave:" }],
    }),
    null,
  );
  assert.deepEqual(
    getWorkflowActionTiles({
      steps: [
        { action: "add_reaction", emoji: "🐥" },
        { action: "send_message", text: "hello" },
        { action: "delay", duration: "5m" },
      ],
    }),
    [
      { action: "add_reaction", emoji: "🐥", key: "step-0" },
      { action: "send_message", emoji: null, key: "step-1" },
      { action: "delay", emoji: null, key: "step-2" },
    ],
  );
});

test("toggles workflow enabled state without mutating its definition", () => {
  const definition = {
    name: "deploy",
    enabled: false,
    trigger: { on: "message_posted" },
    future_field: { keep: true },
  };

  assert.deepEqual(withWorkflowEnabled(definition, true), {
    name: "deploy",
    trigger: { on: "message_posted" },
    future_field: { keep: true },
  });
  assert.deepEqual(withWorkflowEnabled(definition, false), definition);
  assert.equal(definition.enabled, false);
});

test("shows a disabled definition as disabled while preserving other statuses", () => {
  const workflow = {
    id: "workflow-id",
    name: "deploy",
    channelId: "channel-id",
    definition: { enabled: false },
    status: "active",
    createdAt: 1,
    updatedAt: 1,
  };

  assert.equal(getWorkflowDisplayStatus(workflow), "disabled");
  assert.equal(
    getWorkflowDisplayStatus({ ...workflow, status: "archived" }),
    "archived",
  );
});

test("summarizes common and custom schedules", () => {
  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "schedule", interval: "15m" },
      steps: [{ action: "call_webhook" }],
    }),
    "Every 15 minutes, call a webhook",
  );
  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "schedule", cron: "30 9 * * *" },
      steps: [{ action: "request_approval" }],
    }),
    "Every day at 09:30 UTC, request approval",
  );
  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "schedule", cron: "*/5 8-17 * * 1-5" },
      steps: [],
    }),
    "On a custom schedule",
  );
});

test("gracefully labels definitions with future trigger and action types", () => {
  assert.equal(
    getWorkflowCardLabel({
      trigger: { on: "issue_closed" },
      steps: [{ action: "archive_issue" }],
    }),
    "When issue closed happens, archive issue",
  );
  assert.equal(getWorkflowCardLabel({}), "When this workflow starts");
});
