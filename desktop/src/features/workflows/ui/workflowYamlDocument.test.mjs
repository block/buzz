import assert from "node:assert/strict";
import test from "node:test";

import {
  readWorkflowDocumentFields,
  readWorkflowHeaderState,
  yamlWithWorkflowEnabled,
  yamlWithWorkflowName,
} from "./workflowYamlDocument.ts";

/** The name the dialog header would render for `yaml`. */
function headerName(yaml, fallbackName) {
  return readWorkflowHeaderState(yaml, { enabled: true, name: fallbackName })
    .name;
}

/** The enabled state the dialog header would render for `yaml`. */
function headerEnabled(yaml, fallbackEnabled) {
  return readWorkflowHeaderState(yaml, {
    enabled: fallbackEnabled,
    name: undefined,
  }).enabled;
}

// A step that has just been added from the builder carries no message text yet,
// so the definition fails full form validation while the user is still on the
// step pane. The header must keep working against that document.
const INCOMPLETE_STEP_YAML = `name: mock-horse-battery
trigger:
  on: message_posted
steps:
  - id: step_1
    action: send_message
`;

test("reads the name of a definition whose steps are still incomplete", () => {
  assert.deepEqual(readWorkflowDocumentFields(INCOMPLETE_STEP_YAML), {
    editable: true,
    enabled: null,
    name: "mock-horse-battery",
  });
  assert.equal(
    headerName(INCOMPLETE_STEP_YAML, undefined),
    "mock-horse-battery",
  );
});

test("treats an empty definition as an editable blank name", () => {
  assert.deepEqual(readWorkflowDocumentFields(""), {
    editable: true,
    enabled: null,
    name: null,
  });
  assert.deepEqual(readWorkflowDocumentFields("   \n"), {
    editable: true,
    enabled: null,
    name: null,
  });
  assert.equal(headerName("", undefined), "");
});

test("falls back to the saved name only when the document has none", () => {
  assert.equal(headerName("", "Saved name"), "Saved name");
  assert.equal(
    headerName("name: ''\ntrigger:\n  on: webhook\n", "Saved name"),
    "Saved name",
  );
  assert.equal(
    headerName(INCOMPLETE_STEP_YAML, "Saved name"),
    "mock-horse-battery",
  );
});

test("trims the rendered name and the saved fallback", () => {
  assert.equal(headerName('name: "  spaced  "\n', undefined), "spaced");
  assert.equal(headerName("", "  saved  "), "saved");
});

test("marks unparseable or non-map documents as uneditable", () => {
  assert.deepEqual(readWorkflowDocumentFields("name: [unclosed\n"), {
    editable: false,
    enabled: null,
    name: null,
  });
  assert.deepEqual(readWorkflowDocumentFields("- just\n- a list\n"), {
    editable: false,
    enabled: null,
    name: null,
  });
  assert.equal(yamlWithWorkflowName("- just\n- a list\n", "next"), null);
  assert.equal(yamlWithWorkflowEnabled("- just\n- a list\n", false), null);
});

test("ignores a non-string name rather than rendering it", () => {
  assert.equal(readWorkflowDocumentFields("name: 42\n").name, null);
  assert.equal(headerName("name: 42\n", "Saved name"), "Saved name");
});

test("keeps the header editable while a step is still incomplete", () => {
  assert.deepEqual(
    readWorkflowHeaderState(INCOMPLETE_STEP_YAML, {
      enabled: true,
      name: "Saved name",
    }),
    { canEdit: true, enabled: true, name: "mock-horse-battery" },
  );
  assert.equal(
    readWorkflowHeaderState("name: [unclosed\n", {
      enabled: false,
      name: "Saved name",
    }).canEdit,
    false,
  );
});

test("derives enabled from the document, defaulting to enabled", () => {
  assert.equal(headerEnabled(INCOMPLETE_STEP_YAML, false), true);
  assert.equal(
    headerEnabled(`enabled: false\n${INCOMPLETE_STEP_YAML}`, true),
    false,
  );
  // Only an unreadable document may fall back to the saved value.
  assert.equal(headerEnabled("- just\n- a list\n", false), false);
  assert.equal(headerEnabled("- just\n- a list\n", true), true);
});

test("writes the name back without disturbing the rest of the document", () => {
  const next = yamlWithWorkflowName(INCOMPLETE_STEP_YAML, "renamed");
  assert.equal(headerName(next, undefined), "renamed");
  assert.match(next, /action: send_message/);
  assert.match(next, /on: message_posted/);
});

test("seeds a definition when naming an empty document", () => {
  const next = yamlWithWorkflowName("", "fresh");
  assert.equal(headerName(next, undefined), "fresh");
  assert.match(next, /trigger:/);
});

test("adds and removes the enabled key without touching the name", () => {
  const disabled = yamlWithWorkflowEnabled(INCOMPLETE_STEP_YAML, false);
  assert.match(disabled, /enabled: false/);
  assert.equal(headerName(disabled, undefined), "mock-horse-battery");

  const reEnabled = yamlWithWorkflowEnabled(disabled, true);
  assert.doesNotMatch(reEnabled, /enabled:/);
  assert.equal(headerName(reEnabled, undefined), "mock-horse-battery");
});
