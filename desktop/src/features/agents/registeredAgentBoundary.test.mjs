import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import test from "node:test";

const ROOT = new URL("../../..", import.meta.url).pathname;
const SRC = join(ROOT, "src");
const REGISTERED_AGENT_DATA_FILES = new Set([
  "src/features/agents/hooks.ts",
  "src/features/agents/hooksRegistered.test.mjs",
  "src/features/agents/lib/registeredAgentCards.test.mjs",
  "src/features/agents/lib/registeredAgentCards.ts",
  "src/features/agents/lib/useAgentsDataRefresh.ts",
  "src/features/agents/registeredAgentBoundary.test.mjs",
  "src/features/agents/ui/AgentsView.tsx",
  "src/features/agents/ui/RegisterExistingAgentDialog.tsx",
  "src/features/agents/ui/RegisteredAgentIdentityCard.tsx",
  "src/features/agents/ui/RemoveRegisteredAgentDialog.tsx",
  "src/features/agents/ui/UnifiedAgentsSection.tsx",
  "src/features/agents/ui/UnifiedAgentsSectionCardTarget.test.mjs",
  "src/shared/api/registeredAgents.test.mjs",
  "src/shared/api/tauriRegisteredAgents.ts",
  "src/testing/e2eBridge.ts",
]);
const REGISTERED_AGENT_DISPLAY_FILES = new Set([
  "src/features/agents/lib/registeredAgentCards.ts",
  "src/features/agents/ui/RegisterExistingAgentDialog.tsx",
  "src/features/agents/ui/RegisteredAgentIdentityCard.tsx",
  "src/features/agents/ui/RemoveRegisteredAgentDialog.tsx",
  "src/shared/api/tauriRegisteredAgents.ts",
]);

const REGISTERED_AGENT_DATA_MARKERS = [
  "RegisteredAgentReference",
  "listRegisteredAgentReferences",
  "registerExistingAgentReference",
  "registeredAgentsQueryKey",
  "registeredReferences",
  "unregisterExistingAgentReference",
  "useRegisteredAgentsQuery",
];
const FORBIDDEN_TRUST_MARKERS = [
  "KnownAgentPubkeys",
  "configNudgeAuthPubkey",
  "mergeKnownAgentPubkeys",
  "mentionableAgentPubkeys",
  "useKnownAgentPubkeys",
];

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (entry === "node_modules" || entry === "dist") return [];
    if (statSync(path).isDirectory()) return walk(path);
    return /\.(ts|tsx|mjs)$/.test(path) ? [path] : [];
  });
}

test("registered-agent data stays inside the reviewed display/navigation integration files", () => {
  const offenders = [];
  for (const path of walk(SRC)) {
    const rel = relative(ROOT, path);
    const text = readFileSync(path, "utf8");
    const dataHits = REGISTERED_AGENT_DATA_MARKERS.filter((needle) =>
      text.includes(needle),
    );
    if (dataHits.length > 0 && !REGISTERED_AGENT_DATA_FILES.has(rel)) {
      offenders.push(
        `${rel}: registered-reference data (${dataHits.join(", ")})`,
      );
      continue;
    }
    if (
      dataHits.length > 0 &&
      rel !== "src/features/agents/registeredAgentBoundary.test.mjs"
    ) {
      const trustHits = FORBIDDEN_TRUST_MARKERS.filter((needle) =>
        text.includes(needle),
      );
      if (trustHits.length > 0) {
        offenders.push(
          `${rel}: registered-reference trust leak (${trustHits.join(", ")})`,
        );
      }
    }
    if (!REGISTERED_AGENT_DISPLAY_FILES.has(rel)) continue;
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
      "pid",
    ].filter((needle) => text.includes(needle));
    if (forbidden.length > 0) offenders.push(`${rel}: ${forbidden.join(", ")}`);
  }
  assert.deepEqual(offenders, []);
});
