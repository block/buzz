import assert from "node:assert/strict";
import test from "node:test";

import {
  getWorkflowDisplayStatus,
  getWorkflowPrimaryAction,
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

test("updates enabled state without mutating the workflow definition", () => {
  const definition = {
    name: "deploy",
    trigger: { on: "message_posted" },
  };
  const disabled = withWorkflowEnabled(definition, false);

  assert.deepEqual(disabled, { ...definition, enabled: false });
  assert.deepEqual(definition, {
    name: "deploy",
    trigger: { on: "message_posted" },
  });
  assert.deepEqual(withWorkflowEnabled(disabled, true), definition);
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
