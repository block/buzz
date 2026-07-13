/**
 * Effort-table sync guard: TS side.
 *
 * Loads the checked-in fixture and asserts that `getProviderEffortConfig`
 * matches every entry. Drift between `buzzAgentConfig.ts` and the fixture
 * (e.g. a new model family added to one side but not the other) fails CI.
 * The companion Rust test in `crates/buzz-agent/src/config.rs` mirrors
 * this check so both sides of the mirror must stay in sync.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import path from "node:path";

import { getProviderEffortConfig } from "./buzzAgentConfig.ts";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const fixture = JSON.parse(
  readFileSync(path.join(__dirname, "effortTable.fixture.json"), "utf8"),
);

for (const entry of fixture) {
  const {
    note,
    provider,
    model,
    validValues: expectedValidValues,
    defaultValue: expectedDefault,
  } = entry;
  const label = note ?? `${provider}/${model}`;

  test(`effort fixture: ${label}`, () => {
    const { validValues, defaultValue } = getProviderEffortConfig(
      provider,
      model,
    );

    assert.deepEqual(
      [...validValues],
      expectedValidValues,
      `validValues mismatch for "${label}"`,
    );

    assert.equal(
      defaultValue,
      expectedDefault,
      `defaultValue mismatch for "${label}"`,
    );
  });
}
