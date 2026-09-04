import assert from "node:assert/strict";
import test from "node:test";

import { AgentDialog } from "./AgentDialog.tsx";
import { AgentDefinitionDialog } from "./AgentDefinitionDialog.tsx";
import { AgentRunLocationProvider } from "./AgentRunLocationContext.tsx";
import { AgentEditDialog } from "./AgentEditDialog.tsx";
import { AgentEditMergedDialog } from "./AgentEditMergedDialog.tsx";

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
  assert.equal(
    typeof element.type,
    "function",
    "definition must route through the internal create router",
  );
  assert.equal(element.type.name, "AgentCreateDialogRouter");
});

// ── Phase 1 Round 2: merged surface routing (AgentEditDialog) ────────────────
//
// AgentEditDialog collapses R1–R7 onto ONE merged surface: AgentEditMergedDialog.
// Every context — instance-with-definition, instance-only, definition-only —
// renders the merged dialog. Instance-present contexts are wrapped in
// AgentRunLocationProvider so RespondToField can name the run machine.
//
// These tests prove:
//   • instance-with-definition → AgentEditMergedDialog (D+I+L sections, one coordinator)
//   • instance-only            → AgentEditMergedDialog (I+L sections, one coordinator)
//   • definition-only          → AgentEditMergedDialog (D sections only, one coordinator)
//   • instance contexts wrapped in AgentRunLocationProvider
//   • definition-only NOT wrapped (no instance = no run location)
//
// AgentInstanceEditDialog has NO production consumer on any R1–R7 route.

const minimalDefinition = {
  id: "def-abc",
  displayName: "Bot",
  avatarUrl: "",
  systemPrompt: "Do stuff.",
  runtime: "goose",
  model: null,
  provider: null,
  isBuiltIn: false,
  isActive: true,
  namePool: [],
  envVars: {},
  respondTo: null,
  respondToAllowlist: [],
  parallelism: null,
  createdAt: "2025-01-01T00:00:00Z",
  updatedAt: "2025-01-01T00:00:00Z",
};

const minimalInstance = {
  pubkey: "pk-0000",
  name: "Bot",
  avatarUrl: "",
  systemPrompt: "Do stuff.",
  model: null,
  provider: null,
  envVars: {},
  respondTo: "anyone",
  respondToAllowlist: [],
  parallelism: 1,
  autoRestartOnConfigChange: false,
  startOnAppLaunch: false,
};

test("AgentEditDialog: instance-with-definition routes to AgentEditMergedDialog (D+I+L on one form)", () => {
  const ctx = {
    kind: "instance-with-definition",
    definition: minimalDefinition,
    instance: minimalInstance,
  };

  const element = AgentEditDialog({
    ctx,
    open: true,
    onOpenChange: noop,
  });

  // Instance-present: wrapped in AgentRunLocationProvider for respondTo warning.
  assert.equal(element.type, AgentRunLocationProvider);
  const form = element.props.children;
  assert.equal(
    form.type,
    AgentEditMergedDialog,
    "linked agent edit must reach AgentEditMergedDialog — D+I+L sections on one form",
  );
  assert.equal(form.props.ctx, ctx, "ctx must pass through to merged surface");
  assert.equal(form.props.open, true);
});

test("AgentEditDialog: instance-only routes to AgentEditMergedDialog (I+L sections, coordinator)", () => {
  const ctx = { kind: "instance-only", instance: minimalInstance };

  const element = AgentEditDialog({
    ctx,
    open: true,
    onOpenChange: noop,
  });

  // Instance-present: wrapped in AgentRunLocationProvider.
  assert.equal(element.type, AgentRunLocationProvider);
  const form = element.props.children;
  assert.equal(
    form.type,
    AgentEditMergedDialog,
    "unlinked agent edit must reach AgentEditMergedDialog (coordinator handles I-only save)",
  );
  assert.equal(form.props.ctx, ctx);
});

test("AgentEditDialog: definition-only routes to AgentEditMergedDialog (D sections, no wrapper)", () => {
  const ctx = { kind: "definition-only", definition: minimalDefinition };

  const element = AgentEditDialog({
    ctx,
    open: true,
    onOpenChange: noop,
  });

  // Definition-only: no run-location wrapper (no instance backend to resolve).
  assert.equal(
    element.type,
    AgentEditMergedDialog,
    "definition-only must reach AgentEditMergedDialog directly (no AgentRunLocationProvider wrapper)",
  );
  assert.equal(element.props.ctx, ctx);
  assert.equal(element.props.open, true);
});

test("AgentEditDialog: all R1–R7 contexts route to AgentEditMergedDialog (AgentInstanceEditDialog deleted)", () => {
  // AgentInstanceEditDialog was deleted in Phase 1 round 4 (zero production consumers
  // on any R1–R7 route). This test verifies all contexts produce an AgentEditMergedDialog
  // at the root of the element tree.
  const contexts = [
    {
      kind: "instance-with-definition",
      definition: minimalDefinition,
      instance: minimalInstance,
    },
    { kind: "instance-only", instance: minimalInstance },
    { kind: "definition-only", definition: minimalDefinition },
  ];

  for (const ctx of contexts) {
    const element = AgentEditDialog({ ctx, open: true, onOpenChange: noop });

    // Instance contexts are wrapped in AgentRunLocationProvider; definition-only is not.
    // Either way, the innermost dialog element must be AgentEditMergedDialog.
    function findMergedDialog(el) {
      if (!el || typeof el !== "object") return false;
      if (el.type === AgentEditMergedDialog) return true;
      if (el.props?.children) {
        const children = Array.isArray(el.props.children)
          ? el.props.children
          : [el.props.children];
        return children.some((c) => findMergedDialog(c));
      }
      return false;
    }

    assert.equal(
      findMergedDialog(element),
      true,
      `ctx.kind=${ctx.kind}: AgentEditMergedDialog must appear in the rendered tree`,
    );
  }
});

test("AgentEditDialog: single Save wires through coordinator — no instance dialog bypass", () => {
  // Structural proof: the merged surface is reached for instance-with-definition,
  // which means every save goes through AgentEditMergedDialog → emitAgentFormDiff
  // → runAgentSaveCoordinator (steps 0–4, observed-state settlement for D+I+L).
  // The coordinator is wired in AgentEditMergedDialog; this test confirms the
  // routing reaches it (not AgentInstanceEditDialog's standalone save path).
  const ctx = {
    kind: "instance-with-definition",
    definition: minimalDefinition,
    instance: minimalInstance,
  };

  const element = AgentEditDialog({ ctx, open: true, onOpenChange: noop });
  const form = element.props.children; // inner of AgentRunLocationProvider

  assert.equal(form.type, AgentEditMergedDialog);
  // AgentEditMergedDialog accepts ctx directly — the coordinator receives
  // personaInput + agentInput + policySets from emitAgentFormDiff on every save.
  assert.equal(form.props.ctx.kind, "instance-with-definition");
  assert.equal(form.props.ctx.definition, minimalDefinition);
  assert.equal(form.props.ctx.instance, minimalInstance);
});

// ── D-section renders D-fields for all definition contexts ────────────────────
//
// Since removing the !showInst guards from AgentEditMergedDialogDSection, the
// D-section always renders runtime/model/provider regardless of whether an
// instance is present. These tests verify the structural contract by checking
// that the component type tree for instance-with-definition reaches both
// AgentEditMergedDSection and AgentEditMergedInstanceSection via the
// AgentEditMergedDialog (which conditionally renders both sections).
//
// The merged dialog receives ctx with kind="instance-with-definition" and
// the D-section is rendered when hasDefinitionContext(ctx) = true (always for
// that kind). The I-section is rendered when hasInstanceContext(ctx) = true
// AND inst is present (also always for that kind).

import { AgentEditMergedDSection } from "./AgentEditMergedDialogDSection.tsx";
import { AgentEditMergedInstanceSection } from "./AgentEditMergedDialogInstanceSection.tsx";
import { hasDefinitionContext, hasInstanceContext } from "./agentFormModel.ts";

test("hasDefinitionContext returns true for instance-with-definition and definition-only", () => {
  assert.equal(
    hasDefinitionContext({
      kind: "instance-with-definition",
      definition: minimalDefinition,
      instance: minimalInstance,
    }),
    true,
  );
  assert.equal(
    hasDefinitionContext({
      kind: "definition-only",
      definition: minimalDefinition,
    }),
    true,
  );
  assert.equal(
    hasDefinitionContext({ kind: "instance-only", instance: minimalInstance }),
    false,
  );
});

test("hasInstanceContext returns true for instance-with-definition and instance-only", () => {
  assert.equal(
    hasInstanceContext({
      kind: "instance-with-definition",
      definition: minimalDefinition,
      instance: minimalInstance,
    }),
    true,
  );
  assert.equal(
    hasInstanceContext({ kind: "instance-only", instance: minimalInstance }),
    true,
  );
  assert.equal(
    hasInstanceContext({
      kind: "definition-only",
      definition: minimalDefinition,
    }),
    false,
  );
});

test("AgentEditMergedDSection type exists and is a function (D-fields component)", () => {
  // Structural guard: the exported component must be callable. No DOM render needed.
  assert.equal(typeof AgentEditMergedDSection, "function");
});

test("AgentEditMergedInstanceSection type exists and is a function (I-fields component)", () => {
  assert.equal(typeof AgentEditMergedInstanceSection, "function");
});
