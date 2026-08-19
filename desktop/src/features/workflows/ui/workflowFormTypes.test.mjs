import assert from "node:assert/strict";
import test from "node:test";

import { formStateToYaml, yamlToFormState } from "./workflowFormTypes.ts";

test("workflow form preserves send_message reply_to templates", () => {
  const yaml = `
name: Threaded prompt
trigger:
  on: message_posted
steps:
  - id: present_pr
    action: send_message
    text: Present the PR
  - id: show_actions
    action: send_message
    text: Show the actions
    reply_to: "{{steps.present_pr.output.event_id}}"
`;

  const parsed = yamlToFormState(yaml);
  assert.equal(parsed.ok, true);
  if (!parsed.ok) return;

  assert.equal(
    parsed.state.steps[1].replyTo,
    "{{steps.present_pr.output.event_id}}",
  );

  const roundTripped = yamlToFormState(formStateToYaml(parsed.state));
  assert.equal(roundTripped.ok, true);
  if (!roundTripped.ok) return;
  assert.equal(
    roundTripped.state.steps[1].replyTo,
    "{{steps.present_pr.output.event_id}}",
  );
});
