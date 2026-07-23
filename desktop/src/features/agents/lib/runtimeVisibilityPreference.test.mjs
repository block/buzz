import assert from "node:assert/strict";
import test from "node:test";

import {
  ACP_RUNTIME_VISIBILITY_STORAGE_KEY,
  filterEnabledAcpRuntimes,
  maskDisabledAcpRuntimePreference,
  nextDisabledAcpRuntimeIds,
  parseDisabledAcpRuntimeIds,
  readDisabledAcpRuntimeIds,
  runtimesForImplicitAcpSelection,
} from "./runtimeVisibilityPreference.ts";

test("runtime visibility parsing is normalized and corruption tolerant", () => {
  assert.deepEqual(
    parseDisabledAcpRuntimeIds('[" Goose ","codex","goose",4]'),
    ["codex", "goose"],
  );
  assert.deepEqual(parseDisabledAcpRuntimeIds("{not-json"), []);
  assert.deepEqual(parseDisabledAcpRuntimeIds('{"goose":false}'), []);
});

test("enabling and disabling runtimes preserves the other choices", () => {
  const disabled = nextDisabledAcpRuntimeIds([], " Goose ", false);
  assert.deepEqual(disabled, ["goose"]);
  assert.deepEqual(nextDisabledAcpRuntimeIds(disabled, "codex", false), [
    "codex",
    "goose",
  ]);
  assert.deepEqual(nextDisabledAcpRuntimeIds(disabled, "GOOSE", true), []);
});

test("disabled runtimes are removed from selectable catalog entries", () => {
  const runtimes = [
    { id: "buzz-agent", label: "Buzz Agent" },
    { id: "goose", label: "Goose" },
    { id: "codex", label: "Codex" },
  ];

  assert.deepEqual(filterEnabledAcpRuntimes(runtimes, ["Goose"]), [
    runtimes[0],
    runtimes[2],
  ]);
  assert.deepEqual(filterEnabledAcpRuntimes(runtimes, ["buzz-agent"]), [
    runtimes[1],
    runtimes[2],
  ]);
  assert.deepEqual(
    runtimesForImplicitAcpSelection(runtimes, ["buzz-agent"], null),
    [runtimes[1], runtimes[2]],
  );
  assert.deepEqual(
    runtimesForImplicitAcpSelection(runtimes, ["buzz-agent"], "buzz-agent"),
    runtimes,
  );
});

test("stored runtime visibility is read from the versioned device key", () => {
  const storage = {
    getItem(key) {
      assert.equal(key, ACP_RUNTIME_VISIBILITY_STORAGE_KEY);
      return '["claude"]';
    },
  };

  assert.deepEqual(readDisabledAcpRuntimeIds(storage), ["claude"]);
});

test("a disabled saved runtime and its dependent defaults are masked", () => {
  const config = {
    env_vars: {},
    provider: "relay-mesh",
    model: "auto",
    preferred_runtime: "Goose",
  };

  assert.deepEqual(maskDisabledAcpRuntimePreference(config, ["goose"]), {
    ...config,
    provider: null,
    model: null,
    preferred_runtime: null,
  });
  assert.equal(maskDisabledAcpRuntimePreference(config, ["claude"]), config);
});
