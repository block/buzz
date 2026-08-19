import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import test from "node:test";

const ROOT = new URL("../../..", import.meta.url).pathname;
const SRC = join(ROOT, "src");

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (entry === "node_modules" || entry === "dist") return [];
    if (statSync(path).isDirectory()) return walk(path);
    return /\.(ts|tsx|mjs)$/.test(path) ? [path] : [];
  });
}

test("registered-agent frontend does not import or call managed lifecycle/create APIs", () => {
  const offenders = [];
  for (const path of walk(SRC)) {
    const rel = relative(ROOT, path);
    if (!/registeredAgent|RegisteredAgent/.test(rel)) continue;
    if (rel.endsWith("registeredAgentBoundary.test.mjs")) continue;
    if (rel.endsWith("registeredAgents.test.mjs")) continue;
    const text = readFileSync(path, "utf8");
    const forbidden = [
      "createManagedAgent",
      "startManagedAgent",
      "stopManagedAgent",
      "deleteManagedAgent",
      "managedAgentRuntime",
      "privateKeyNsec",
      "private_key_nsec",
      "envVars",
      "agentCommand",
      "agent_command",
    ].filter((needle) => text.includes(needle));
    if (forbidden.length > 0) offenders.push(`${rel}: ${forbidden.join(", ")}`);
  }
  assert.deepEqual(offenders, []);
});
