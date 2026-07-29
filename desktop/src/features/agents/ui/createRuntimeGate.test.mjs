import assert from "node:assert/strict";
import test from "node:test";

import {
  createGateHarnessId,
  createRuntimeIsAvailable,
  createRuntimeOptionDisabled,
  createRuntimeSeedAction,
  createRuntimeSeedAllowed,
  createRuntimeSelectionSatisfied,
  runtimeDropdownOptions,
  runtimeDropdownPlaceholder,
} from "./createRuntimeGate.ts";
import { requiredCredentialEnvKeys } from "./agentConfigOptions.tsx";

function runtimeEntry(overrides = {}) {
  return {
    id: "buzz-agent",
    label: "Buzz Agent",
    availability: "available",
    requiresExternalCli: false,
    installHint: "",
    ...overrides,
  };
}

function gate(overrides = {}) {
  return {
    isCreateMode: true,
    runsRemotely: false,
    runtime: "buzz-agent",
    selectedRuntime: runtimeEntry(),
    hasLocalDefaultRuntime: true,
    ...overrides,
  };
}

test("a local create requires a locally-available runtime", () => {
  assert.equal(createRuntimeSelectionSatisfied(gate()), true);
  assert.equal(
    createRuntimeSelectionSatisfied(
      gate({
        selectedRuntime: runtimeEntry({ availability: "not_installed" }),
      }),
    ),
    false,
  );
  assert.equal(
    createRuntimeSelectionSatisfied(gate({ runtime: "  " })),
    false,
    "a local create must name a runtime",
  );
});

test("a remote create is not gated by the local catalog", () => {
  const remote = gate({
    runsRemotely: true,
    runtime: "",
    selectedRuntime: null,
  });
  assert.equal(createRuntimeSelectionSatisfied(remote), true);
  assert.equal(createRuntimeIsAvailable(remote), true);
  assert.equal(
    createRuntimeIsAvailable(
      gate({
        runsRemotely: true,
        selectedRuntime: runtimeEntry({ availability: "not_installed" }),
      }),
    ),
    true,
  );
});

test("edit mode never applies the create-only runtime requirement", () => {
  assert.equal(
    createRuntimeSelectionSatisfied(
      gate({ isCreateMode: false, runtime: "", selectedRuntime: null }),
    ),
    true,
  );
});

test("unavailable options are disabled only for a gated local create", () => {
  const missing = runtimeEntry({ id: "goose", availability: "not_installed" });
  assert.equal(createRuntimeOptionDisabled(missing, gate()), true);
  assert.equal(
    createRuntimeOptionDisabled(missing, gate({ runsRemotely: true })),
    false,
    "the remote host's catalog decides availability, not this machine's",
  );
  assert.equal(
    createRuntimeOptionDisabled(missing, gate({ isCreateMode: false })),
    false,
  );
  assert.equal(
    createRuntimeOptionDisabled(
      missing,
      gate({ hasLocalDefaultRuntime: false }),
    ),
    false,
    "with nothing installed, disabling every option would trap the user",
  );
});

test("the dropdown offers a blank option only outside create mode", () => {
  const runtimes = [runtimeEntry()];
  const created = runtimeDropdownOptions({
    defaultRuntimeId: "buzz-agent",
    gate: gate(),
    runtimes,
    runtimesLoading: false,
  });
  assert.deepEqual(
    created.map((option) => option.value),
    ["buzz-agent"],
  );
  assert.equal(created[0].label, "Buzz Agent (default)");

  const edited = runtimeDropdownOptions({
    defaultRuntimeId: "buzz-agent",
    gate: gate({ isCreateMode: false }),
    runtimes,
    runtimesLoading: false,
  });
  assert.deepEqual(
    edited.map((option) => option.value),
    ["__no_runtime__", "buzz-agent"],
  );
  assert.equal(edited[1].label, "Buzz Agent", "no default marker when editing");
});

test("a runtime the catalog no longer knows keeps its own entry", () => {
  const options = runtimeDropdownOptions({
    defaultRuntimeId: "buzz-agent",
    gate: gate({ isCreateMode: false, runtime: "retired-harness" }),
    runtimes: [runtimeEntry()],
    runtimesLoading: false,
  });
  assert.deepEqual(options.at(-1), {
    label: "retired-harness (current)",
    value: "retired-harness",
  });
});

test("remote dropdown options are all selectable", () => {
  const options = runtimeDropdownOptions({
    defaultRuntimeId: "buzz-agent",
    gate: gate({ runsRemotely: true }),
    runtimes: [runtimeEntry({ id: "goose", availability: "not_installed" })],
    runtimesLoading: false,
  });
  assert.equal(options[0].disabled, false);
});

test("the placeholder tracks loading and mode", () => {
  assert.equal(
    runtimeDropdownPlaceholder({ isCreateMode: true, runtimesLoading: true }),
    "Loading harnesses...",
  );
  assert.equal(
    runtimeDropdownPlaceholder({ isCreateMode: true, runtimesLoading: false }),
    "Choose a harness",
  );
  assert.equal(
    runtimeDropdownPlaceholder({ isCreateMode: false, runtimesLoading: false }),
    "No preference (use app default)",
  );
});

test("a local create asks the local runtime for its credential keys", () => {
  assert.equal(
    createGateHarnessId({
      runsRemotely: false,
      runtime: "buzz-agent",
      remoteHarnessId: "goose",
    }),
    "buzz-agent",
    "a stale remote pin never leaks into a local create",
  );
});

test("a remote goose on a buzz-agent laptop demands GOOSE_*, not BUZZ_AGENT_*", () => {
  // The bug this guards: `runtime` is seeded from the LOCAL catalog, so a
  // machine defaulting to buzz-agent would demand BUZZ_AGENT-shaped
  // credentials for an agent that runs Goose on someone else's host.
  const harnessId = createGateHarnessId({
    runsRemotely: true,
    runtime: "buzz-agent",
    remoteHarnessId: "goose",
  });
  assert.equal(harnessId, "goose");
  assert.deepEqual(requiredCredentialEnvKeys(harnessId, "anthropic"), [
    "ANTHROPIC_API_KEY",
  ]);
});

test("a remote goose on a claude laptop still demands credentials", () => {
  // The other half of the same bug: claude/codex support no provider
  // selection, so the local id made the requirement list empty and the create
  // shipped with no provider, model, or key at all.
  assert.deepEqual(requiredCredentialEnvKeys("claude", "anthropic"), []);
  assert.deepEqual(
    requiredCredentialEnvKeys(
      createGateHarnessId({
        runsRemotely: true,
        runtime: "claude",
        remoteHarnessId: "goose",
      }),
      "anthropic",
    ),
    ["ANTHROPIC_API_KEY"],
  );
});

// ── the harness auto-seed is local-only ─────────────────────────────────────

function seedInput(overrides = {}) {
  return {
    defaultRuntimeId: "buzz-agent",
    definitionRuntime: undefined,
    hasInitialValues: true,
    hasSeededForOpen: false,
    isAutoSeeded: false,
    open: true,
    runsRemotely: false,
    runtime: "",
    runtimesLoading: false,
    ...overrides,
  };
}

test("a local create still seeds this computer's default harness", () => {
  assert.deepEqual(createRuntimeSeedAction(seedInput()), {
    type: "seed",
    runtimeId: "buzz-agent",
  });
});

// The defect: a provider-backed agent takes its harness from the HOST's
// catalog, so stamping the local default onto the draft describes the wrong
// machine — the edit dialog then reports a remote SSH agent as running
// "Buzz Agent" locally.
test("a remote create is never seeded with the local default", () => {
  assert.deepEqual(createRuntimeSeedAction(seedInput({ runsRemotely: true })), {
    type: "none",
  });
});

// Refusing to seed is not enough on its own: "Where to run" starts local and
// lives inside the dialog, so the seed has already been applied by the time the
// user picks a provider. Without the shed, the remote create submits it anyway.
test("switching to a provider sheds an already-seeded local default", () => {
  assert.deepEqual(
    createRuntimeSeedAction(
      seedInput({
        runsRemotely: true,
        runtime: "buzz-agent",
        isAutoSeeded: true,
        hasSeededForOpen: true,
      }),
    ),
    { type: "shed" },
  );
});

test("a harness the user picked explicitly is never shed", () => {
  assert.deepEqual(
    createRuntimeSeedAction(
      seedInput({
        runsRemotely: true,
        runtime: "goose",
        isAutoSeeded: false,
        hasSeededForOpen: true,
      }),
    ),
    { type: "none" },
    "an explicit pick belongs to the user, remote or not",
  );
});

test("the seed never overrides a definition's own runtime or a loaded catalog", () => {
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ definitionRuntime: "goose" })),
    { type: "none" },
  );
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ runtimesLoading: true })),
    { type: "none" },
  );
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ defaultRuntimeId: null })),
    { type: "none" },
    "nothing installed locally means nothing to seed",
  );
  assert.deepEqual(createRuntimeSeedAction(seedInput({ open: false })), {
    type: "none",
  });
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ hasSeededForOpen: true })),
    { type: "none" },
    "the seed fires at most once per dialog-open",
  );
});

// The edit-mode half of the same defect. `runsRemotely` comes from the "Where
// to run" control, which exists only during a create — so it is false here and
// the seed happily stamped this computer's default onto a definition whose
// record runs on a host. The blank runtime is deliberate: `to_definition_view`
// drops the harness because the real one is the host's.
test("editing a provider record never seeds the local default", () => {
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ editsProviderRecord: true })),
    { type: "none" },
    "a blank definition runtime on a remote record is not an absence to fill",
  );
});

test("an already-seeded provider-record edit sheds the local default", () => {
  assert.deepEqual(
    createRuntimeSeedAction(
      seedInput({
        editsProviderRecord: true,
        runtime: "buzz-agent",
        isAutoSeeded: true,
        hasSeededForOpen: true,
      }),
    ),
    { type: "shed" },
    "the phantom seed is what armed needsProviderSelection and blocked Save",
  );
});

test("a local edit is untouched by the provider-record guard", () => {
  assert.deepEqual(
    createRuntimeSeedAction(seedInput({ editsProviderRecord: false })),
    { type: "seed", runtimeId: "buzz-agent" },
  );
});

// The guard is EDIT-only, and its call site is where that can be got wrong:
// the profile panel drives ONE dialog from three handlers -- Edit agent, Edit
// persona and Duplicate -- and only the first two seed an `id`. Handing the
// panel's provider-backed answer straight through armed the guard on a CREATE,
// where the sibling effect in `useCreateRuntimeSeed` (create-only: it bails on
// `"id" in initialValues`) immediately put the shed harness back. The two
// effects then swapped the same field forever -- "Maximum update depth
// exceeded" on the usual remote shape, where the duplicate's model and
// provider are both blank.
test("duplicating a provider-backed persona seeds like any other local create", () => {
  // A duplicate arrives with no harness of its own: `to_definition_view` drops
  // the host's, so the persona it copies has none to carry over.
  const duplicate = seedInput({
    definitionRuntime: undefined,
    editsProviderRecord: false,
  });
  assert.deepEqual(createRuntimeSeedAction(duplicate), {
    type: "seed",
    runtimeId: "buzz-agent",
  });
  assert.deepEqual(
    createRuntimeSeedAction({
      ...duplicate,
      runtime: "buzz-agent",
      isAutoSeeded: true,
      hasSeededForOpen: true,
    }),
    { type: "none" },
    "the seed settles in one pass -- nothing sheds what the create just seeded",
  );
});

// Why the fix belongs at the call site and not inside this function: the two
// effects have no shared state to negotiate with, so they can only agree by
// asking the same question. On a create -- the only shape the sibling effect
// acts on -- the shed must therefore turn on `runsRemotely` alone, which is
// exactly what that effect's own `createRuntimeSeedAllowed` bail reads.
test("on a create, only 'Where to run' can shed -- the same fact the sibling effect bails on", () => {
  for (const runsRemotely of [false, true]) {
    for (const isAutoSeeded of [false, true]) {
      const action = createRuntimeSeedAction(
        seedInput({
          // Guaranteed by the call site: a create has no `id`, so it never
          // carries the edit-only provider-record answer.
          editsProviderRecord: false,
          hasSeededForOpen: isAutoSeeded,
          isAutoSeeded,
          runsRemotely,
          runtime: isAutoSeeded ? "buzz-agent" : "",
        }),
      );
      const siblingWouldSeed = createRuntimeSeedAllowed(runsRemotely);
      assert.equal(
        action.type === "shed",
        !siblingWouldSeed && isAutoSeeded,
        `runsRemotely=${runsRemotely} isAutoSeeded=${isAutoSeeded}: a shed is only ever legal where the sibling effect has already bailed`,
      );
    }
  }
});

test("an unpinned remote harness demands nothing yet", () => {
  const harnessId = createGateHarnessId({
    runsRemotely: true,
    runtime: "buzz-agent",
    remoteHarnessId: null,
  });
  assert.equal(harnessId, "");
  assert.deepEqual(requiredCredentialEnvKeys(harnessId, "anthropic"), []);
});
