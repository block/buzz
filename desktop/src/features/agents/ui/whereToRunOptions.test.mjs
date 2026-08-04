import assert from "node:assert/strict";
import test from "node:test";

import { executionNodeRunOnValue } from "./whereToRunIntent.ts";
import { deriveRunOnOptions } from "./whereToRunOptions.ts";

function node(overrides = {}) {
  return {
    availability: "connected",
    capabilities: ["deploy"],
    displayName: "Build box",
    nodeId: "0123456789abcdef0123456789abcdef",
    ...overrides,
  };
}

function derive(overrides = {}) {
  return deriveRunOnOptions({
    backendProviders: [],
    executionNodes: [],
    providersEnabled: false,
    runOn: "local",
    ...overrides,
  });
}

test("local is always the first card and always selectable", () => {
  const options = derive();
  assert.equal(options.length, 1);
  assert.deepEqual(options[0], {
    availability: "connected",
    detail: null,
    kind: "local",
    label: "This computer",
    selectable: true,
    value: "local",
  });
});

test("deploy-capable nodes become cards with their availability", () => {
  const options = derive({
    executionNodes: [
      node(),
      node({
        availability: "degraded",
        displayName: "Flaky box",
        nodeId: "feed0000feed0000feed0000feed0000",
      }),
    ],
  });
  assert.deepEqual(
    options.slice(1).map((option) => ({
      availability: option.availability,
      label: option.label,
      selectable: option.selectable,
      value: option.value,
    })),
    [
      {
        availability: "connected",
        label: "Build box",
        selectable: true,
        value: executionNodeRunOnValue("0123456789abcdef0123456789abcdef"),
      },
      {
        availability: "degraded",
        label: "Flaky box",
        selectable: true,
        value: executionNodeRunOnValue("feed0000feed0000feed0000feed0000"),
      },
    ],
  );
});

test("nodes without the deploy capability are excluded", () => {
  const options = derive({
    executionNodes: [node({ capabilities: ["observe"] })],
  });
  assert.equal(options.length, 1);
});

test("unavailable nodes stay visible but are not selectable", () => {
  const options = derive({
    executionNodes: [node({ availability: "unavailable" })],
  });
  assert.equal(options[1].availability, "unavailable");
  assert.equal(options[1].selectable, false);
});

test("providers appear only when the provider path is enabled", () => {
  const providers = [{ id: "blox" }];
  assert.equal(
    derive({ backendProviders: providers, providersEnabled: false }).length,
    1,
  );
  const options = derive({
    backendProviders: providers,
    providersEnabled: true,
  });
  assert.deepEqual(options[1], {
    availability: null,
    detail: null,
    kind: "provider",
    label: "blox",
    selectable: true,
    value: "blox",
  });
});

test("a selected node that stopped announcing gets a current-unavailable fallback card", () => {
  const runOn = executionNodeRunOnValue("0123456789abcdef0123456789abcdef");
  const options = derive({ runOn });
  assert.deepEqual(options[1], {
    availability: "unavailable",
    detail: "current, unavailable",
    kind: "execution-node",
    label: "Node 01234567…",
    selectable: true,
    value: runOn,
  });
});

test("no fallback card is added when the selected node is still announced", () => {
  const runOn = executionNodeRunOnValue(node().nodeId);
  const options = derive({ executionNodes: [node()], runOn });
  assert.equal(options.length, 2);
  assert.equal(options[1].detail, null);
});

test("a selected provider that is no longer discovered gets a current fallback card", () => {
  const options = derive({
    backendProviders: [{ id: "other" }],
    providersEnabled: true,
    runOn: "blox",
  });
  assert.deepEqual(options.at(-1), {
    availability: null,
    detail: "current",
    kind: "provider",
    label: "blox",
    selectable: true,
    value: "blox",
  });
});

test("the fallback also covers providers hidden by the provider gate", () => {
  const options = derive({
    backendProviders: [{ id: "blox" }],
    providersEnabled: false,
    runOn: "blox",
  });
  assert.equal(options.length, 2);
  assert.equal(options[1].detail, "current");
});
