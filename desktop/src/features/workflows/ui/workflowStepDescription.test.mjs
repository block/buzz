import assert from "node:assert/strict";
import test from "node:test";

import { workflowStepDescription } from "./workflowStepDescription.ts";

test("describes configured workflow steps on the canvas", () => {
  assert.equal(
    workflowStepDescription({
      id: "message",
      action: "send_message",
      name: "say hello",
      text: "hey yourself",
    }),
    "say hello · “hey yourself”",
  );
  assert.equal(
    workflowStepDescription(
      {
        id: "channel-message",
        action: "send_message",
        text: "deploying now",
        channel: "channel-id",
      },
      { channelLabel: "deployments" },
    ),
    "“deploying now” in #deployments",
  );
  assert.equal(
    workflowStepDescription({
      id: "delay",
      action: "delay",
      duration: "5m",
    }),
    "5 minutes",
  );
  assert.equal(
    workflowStepDescription({
      id: "long-delay",
      action: "delay",
      duration: "4w",
    }),
    "4 weeks",
  );
  assert.equal(
    workflowStepDescription({
      id: "webhook",
      action: "call_webhook",
      method: "PATCH",
      url: "https://example.com/deploy",
    }),
    "PATCH https://example.com/deploy",
  );
  assert.equal(
    workflowStepDescription({
      id: "dm",
      action: "send_dm",
      text: "Build finished",
      to: "team-on-call",
    }),
    "“Build finished” to team-on-call",
  );
  assert.equal(
    workflowStepDescription({
      id: "approval",
      action: "request_approval",
      message: "Ship the release?",
      from: "release-managers",
    }),
    "“Ship the release?” from release-managers",
  );
  assert.equal(
    workflowStepDescription({
      id: "reaction",
      action: "add_reaction",
      emoji: ":buzz:",
    }),
    ":buzz:",
  );
  assert.equal(
    workflowStepDescription({
      id: "topic",
      action: "set_channel_topic",
      topic: "Shipping this week",
    }),
    "“Shipping this week”",
  );
});

test("falls back to the action label until a step is configured", () => {
  assert.equal(
    workflowStepDescription({ id: "message", action: "send_message" }),
    "Send Message",
  );
});

test("can omit a custom step name when composing an index label", () => {
  assert.equal(
    workflowStepDescription(
      {
        id: "message",
        action: "send_message",
        name: "say hello",
        text: "hey yourself",
      },
      { includeName: false },
    ),
    "“hey yourself”",
  );
});
