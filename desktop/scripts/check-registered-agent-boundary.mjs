#!/usr/bin/env node
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.cwd();
const src = join(root, "src");
// Registered references are a display/navigation-only data source. Keep the
// complete consumer list explicit: a new integration file must be reviewed
// and added here instead of evading the check because its filename does not
// happen to contain "registeredAgent".
const registeredAgentDataFiles = new Set([
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
const registeredAgentDisplayFiles = new Set([
  "src/features/agents/lib/registeredAgentCards.ts",
  "src/features/agents/ui/RegisterExistingAgentDialog.tsx",
  "src/features/agents/ui/RegisteredAgentIdentityCard.tsx",
  "src/features/agents/ui/RemoveRegisteredAgentDialog.tsx",
  "src/shared/api/tauriRegisteredAgents.ts",
]);
const registeredAgentDataMarkers = [
  "RegisteredAgentReference",
  "listRegisteredAgentReferences",
  "registerExistingAgentReference",
  "registeredAgentsQueryKey",
  "registeredReferences",
  "unregisterExistingAgentReference",
  "useRegisteredAgentsQuery",
];
const forbiddenTrustMarkers = [
  "KnownAgentPubkeys",
  "configNudgeAuthPubkey",
  "mergeKnownAgentPubkeys",
  "mentionableAgentPubkeys",
  "useKnownAgentPubkeys",
];
const forbiddenInRegisteredAgentFiles = [
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
];

function walk(dir) {
  return readdirSync(dir).flatMap((entry) => {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) return walk(path);
    return /\.(ts|tsx)$/.test(path) ? [path] : [];
  });
}

const offenders = [];
for (const path of walk(src)) {
  const rel = relative(root, path);
  const text = readFileSync(path, "utf8");
  const dataHits = registeredAgentDataMarkers.filter((needle) =>
    text.includes(needle),
  );
  if (dataHits.length > 0 && !registeredAgentDataFiles.has(rel)) {
    offenders.push(
      `${rel}: registered-reference data (${dataHits.join(", ")})`,
    );
  }
  if (dataHits.length > 0 && registeredAgentDataFiles.has(rel)) {
    const trustHits = forbiddenTrustMarkers.filter((needle) =>
      text.includes(needle),
    );
    if (trustHits.length > 0) {
      offenders.push(
        `${rel}: registered-reference trust leak (${trustHits.join(", ")})`,
      );
    }
  }
  if (!registeredAgentDisplayFiles.has(rel)) continue;
  const forbiddenHits = forbiddenInRegisteredAgentFiles.filter((needle) =>
    text.includes(needle),
  );
  if (forbiddenHits.length > 0) {
    offenders.push(`${rel}: ${forbiddenHits.join(", ")}`);
  }
}

if (offenders.length > 0) {
  console.error("Registered agent boundary violations:");
  for (const offender of offenders) console.error(`- ${offender}`);
  process.exit(1);
}
console.log("Registered agent boundary OK");
