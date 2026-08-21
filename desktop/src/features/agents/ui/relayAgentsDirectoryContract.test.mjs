import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

function read(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

test("agents page is backed by accessible relay agents, not local demos", () => {
  const source = read("./AgentsView.tsx");

  assert.match(source, /useRelayAgentsQuery\(\)/);
  assert.match(source, /availableRelayAgents\(/);
  assert.match(source, /<RelayAgentsSection/);
  assert.doesNotMatch(source, /Fizz|Honey|Pollen|Welcome Team/);
  assert.doesNotMatch(source, /list_managed_agents/);
});

test("huddle chooser lists relay agents and keeps enrolled agents visible", () => {
  const source = read("../../huddle/components/AddAgentDialog.tsx");

  assert.match(source, /useRelayAgentsQuery\(\{ enabled: open \}\)/);
  assert.match(source, /availableRelayAgents\(/);
  assert.match(source, /Already in huddle/);
  assert.match(source, /disabled=\{adding !== null \|\| isCurrent\}/);
  assert.doesNotMatch(source, /list_managed_agents/);
});
