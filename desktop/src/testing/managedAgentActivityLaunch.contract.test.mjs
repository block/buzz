import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const remoteLaunchSource = readFileSync(
  new URL("../../src-tauri/src/commands/agents_deploy.rs", import.meta.url),
  "utf8",
);
const localLaunchSource = readFileSync(
  new URL("../../src-tauri/src/managed_agents/runtime.rs", import.meta.url),
  "utf8",
);

function occurrences(source, fragment) {
  return source.split(fragment).length - 1;
}

test("managed-agent launch paths enable the member-safe activity publisher", () => {
  assert.equal(
    occurrences(
      remoteLaunchSource,
      'policy_env.insert("BUZZ_ACP_RELAY_ACTIVITY".into(), "true".into())',
    ),
    1,
    "remote launch policy must enable sanitized activity exactly once",
  );
  assert.equal(
    occurrences(
      localLaunchSource,
      'command.env("BUZZ_ACP_RELAY_ACTIVITY", "true")',
    ),
    1,
    "local launch policy must enable sanitized activity exactly once",
  );
});
