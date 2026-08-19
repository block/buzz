import assert from "node:assert/strict";
import test from "node:test";

import { AgentDialog } from "./AgentDialog.tsx";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog.tsx";
import { AgentInstanceEditDialog } from "./AgentInstanceEditDialog.tsx";
import { AgentRunLocationProvider } from "./AgentRunLocationContext.tsx";

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
    isPending: false,
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

  // The arm wraps the form in the run-location provider so the respond-to
  // warning can name the machine without the value being threaded as a prop
  // through AgentInstanceEditDialog (see AgentRunLocationContext for why).
  assert.equal(element.type, AgentRunLocationProvider);
  const form = element.props.children;
  assert.equal(form.type, AgentInstanceEditDialog);
  assert.deepEqual(form.props, {
    agent,
    onEditLinkedPersona: undefined,
    onOpenChange,
    onUpdated,
    open: true,
    initialFocus: undefined,
  });
});

test("instance-edit publishes the run location resolved from the agent backend", () => {
  const routeWithBackend = (backend) =>
    AgentDialog({
      mode: "instance-edit",
      agent: { pubkey: "abc", name: "test-agent", backend },
      onOpenChange: noop,
      onUpdated: noop,
      open: true,
    }).props.runLocation;

  assert.equal(routeWithBackend({ type: "local" }), "local");
  assert.equal(
    routeWithBackend({ type: "provider", id: "blox", config: {} }),
    "remote",
  );
  // An agent with no backend record has an unknown location — never a guess.
  assert.equal(routeWithBackend(undefined), null);
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
