import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveModelDiscovery,
  shouldSuppressLocalDiscovery,
} from "./useRemoteAwareModelDiscovery.ts";

const localView = {
  discoveredModelOptions: [{ id: "local-model", label: "Local model" }],
  modelDiscoveryLoading: false,
  modelDiscoveryStatus: null,
};

function remoteView(overrides = {}) {
  return {
    harnessId: "goose",
    discoveredModelOptions: [{ id: "host-model", label: "Host model" }],
    modelDiscoveryLoading: false,
    modelDiscoveryStatus: null,
    ...overrides,
  };
}

test("a local create keeps this computer's catalog", () => {
  assert.equal(resolveModelDiscovery(null, localView), localView);
  assert.equal(shouldSuppressLocalDiscovery(null), false);
});

// The whole point of the remote path: the two catalogs describe different
// computers, so a union would offer models the chosen harness cannot run.
test("the host's catalog replaces the local one outright, never merges", () => {
  const remote = remoteView();
  const resolved = resolveModelDiscovery(remote, localView);

  assert.deepEqual(resolved.discoveredModelOptions, [
    { id: "host-model", label: "Host model" },
  ]);
  assert.equal(
    resolved.discoveredModelOptions.some(
      (option) => option.id === "local-model",
    ),
    false,
    "this computer's models must not reach a remote harness",
  );
});

// A remote probe still in flight, or one that failed, describes the HOST. The
// local catalog must not fill the gap — it would silently answer for the wrong
// machine at exactly the moment the user is choosing a model.
test("an in-flight or failed host probe does not fall back to local models", () => {
  const loading = resolveModelDiscovery(
    remoteView({ discoveredModelOptions: null, modelDiscoveryLoading: true }),
    localView,
  );
  assert.equal(loading.discoveredModelOptions, null);
  assert.equal(loading.modelDiscoveryLoading, true);

  const failed = resolveModelDiscovery(
    remoteView({
      discoveredModelOptions: null,
      modelDiscoveryStatus: { message: "ssh: refused", tone: "warning" },
    }),
    localView,
  );
  assert.equal(failed.discoveredModelOptions, null);
  assert.equal(failed.modelDiscoveryStatus.tone, "warning");
});

// Local discovery is IPC that runs a CLI on this machine using the user's
// credentials. Under a live remote catalog its answer can never be rendered,
// so running it at all is noise plus needless credential use.
test("local discovery is suppressed entirely while the host owns the control", () => {
  assert.equal(shouldSuppressLocalDiscovery(remoteView()), true);
  assert.equal(
    shouldSuppressLocalDiscovery(
      remoteView({ discoveredModelOptions: null, modelDiscoveryLoading: true }),
    ),
    true,
    "suppression starts as soon as a harness is probed, not once it answers",
  );
});
