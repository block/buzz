import assert from "node:assert/strict";
import { test } from "node:test";

import {
  activeTemplateToken,
  insertTemplateVariable,
  workflowTemplateVariables,
} from "./workflowTemplateVariables.ts";

test("finds the active template token at the caret", () => {
  assert.deepEqual(activeTemplateToken("hello {{trig", 12), {
    start: 6,
    end: 12,
    query: "trig",
  });
  assert.equal(activeTemplateToken("hello {{trigger.text}}", 22), null);
  assert.equal(activeTemplateToken("hello {{trigger ", 16), null);
});

test("inserts a selected variable and preserves surrounding text", () => {
  const token = activeTemplateToken("before {{tri after", 12);
  assert.ok(token);
  assert.deepEqual(
    insertTemplateVariable("before {{tri after", token, "trigger.text"),
    {
      value: "before {{trigger.text}} after",
      caret: 23,
    },
  );
});

test("replaces the whole template when editing inside an existing token", () => {
  const token = activeTemplateToken("{{trigger.text}}", 9);
  assert.ok(token);
  assert.deepEqual(
    insertTemplateVariable("{{trigger.text}}", token, "trigger.author"),
    {
      value: "{{trigger.author}}",
      caret: 18,
    },
  );
});

test("offers trigger fields and known previous-step outputs", () => {
  const variables = workflowTemplateVariables("message_posted", [
    { id: "notify", action: "send_message" },
    { id: "pause", action: "delay" },
  ]).map(({ value }) => value);

  assert.ok(variables.includes("trigger.text"));
  assert.ok(variables.includes("trigger.author"));
  assert.ok(variables.includes("steps.notify.output.event_id"));
  assert.ok(variables.includes("steps.pause.output.slept_secs"));
});

test("offers only outputs available in every runtime shape", () => {
  const variables = workflowTemplateVariables("reaction_added", [
    { id: "react", action: "add_reaction" },
  ]).map(({ value }) => value);

  assert.ok(variables.includes("trigger.emoji"));
  assert.ok(variables.includes("steps.react.output.added"));
  assert.ok(!variables.includes("steps.react.output.status"));
  assert.ok(!variables.includes("steps.react.output.response"));
});

test("keeps prior-step ordering and ignores unsafe step IDs", () => {
  const variables = workflowTemplateVariables("webhook", [
    { id: "first", action: "delay" },
    { id: "unsafe-id", action: "send_message" },
    { id: "second", action: "send_message" },
  ]).map(({ value }) => value);

  assert.deepEqual(variables, [
    "trigger.channel_id",
    "steps.first.output.slept_secs",
    "steps.second.output.sent",
    "steps.second.output.event_id",
  ]);
});

test("only offers values populated by the selected trigger", () => {
  const variables = workflowTemplateVariables("schedule", []).map(
    ({ value }) => value,
  );
  assert.deepEqual(variables, ["trigger.channel_id", "trigger.timestamp"]);
});
