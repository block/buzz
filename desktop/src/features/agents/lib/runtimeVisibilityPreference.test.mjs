import assert from "node:assert/strict";
import test from "node:test";

import {
  ACP_RUNTIME_VISIBILITY_STORAGE_KEY,
  filterEnabledAcpRuntimes,
  nextDisabledAcpRuntimeIds,
  parseDisabledAcpRuntimeIds,
  readDisabledAcpRuntimeIds,
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
