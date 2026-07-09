import assert from "node:assert/strict";
import test from "node:test";

import { AgentDialog } from "./AgentDialog.tsx";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog.tsx";
import { AgentInstanceEditDialog } from "./AgentInstanceEditDialog.tsx";

// ── Phase 1B.3c routing pinning ─────────────────────────────────────────────
//
// AgentDialog is the single dialog entry point for every intent. It is
// hook-free by design, so each union arm can be exercised as a plain function
// call and the returned element inspected: the arm must route to the form
// component that owns the intent, and pass-through arms must forward the
// caller's props byte-for-byte (minus the `mode` discriminant).

const noop = () => {};

test("definition-edit routes to AgentDefinitionDialog with exact pass-through", () => {
  const props = {
    description: "Edit the agent definition.",
    error: null,
    initialValues: { displayName: "Brain" },
    isImportPending: true,
    isPending: false,
    onImportUpdateFile: async () => {},
    onOpenChange: noop,
    onSubmit: async () => {},
    open: true,
    runtimes: [],
    runtimesLoading: false,
    submitLabel: "Save",
    title: "Edit agent",
  };

  const element = AgentDialog({ mode: "definition-edit", ...props });

  assert.equal(element.type, AgentDefinitionDialog);
  assert.deepEqual(element.props, props, "props must pass through unchanged");
  assert.equal(
    "mode" in element.props,
    false,
    "the mode discriminant must not leak into AgentDefinitionDialog",
  );
});

test("instance-edit routes to AgentInstanceEditDialog with its contract props", () => {
  const agent = { pubkey: "abc", name: "test-agent" };
  const onOpenChange = noop;
  const onUpdated = noop;

  const element = AgentDialog({
    mode: "instance-edit",
    agent,
    onOpenChange,
    onUpdated,
    open: true,
  });

  assert.equal(element.type, AgentInstanceEditDialog);
  assert.deepEqual(element.props, {
    agent,
    onOpenChange,
    onUpdated,
    open: true,
  });
});

test("create mode routes to the internal create router, not a form directly", () => {
  const element = AgentDialog({
    mode: "definition",
    definitionError: null,
    isDefinitionPending: false,
    onOpenChange: noop,
    onSubmitDefinition: async () => true,
    runtimes: [],
    runtimesLoading: false,
  });

  assert.notEqual(element.type, AgentDefinitionDialog);
  assert.notEqual(element.type, AgentInstanceEditDialog);
  assert.equal(
    typeof element.type,
    "function",
    "definition must route through the internal create router",
  );
  assert.equal(element.type.name, "AgentCreateDialogRouter");
});
