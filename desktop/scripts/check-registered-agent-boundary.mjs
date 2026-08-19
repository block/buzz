#!/usr/bin/env node
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.cwd();
const src = join(root, "src");
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
  if (!/registeredAgent|RegisteredAgent/.test(rel)) continue;
  const text = readFileSync(path, "utf8");
  const hits = forbidden.filter((needle) => text.includes(needle));
  if (hits.length > 0) offenders.push(`${rel}: ${hits.join(", ")}`);
}

if (offenders.length > 0) {
  console.error("Registered agent boundary violations:");
  for (const offender of offenders) console.error(`- ${offender}`);
  process.exit(1);
}
console.log("Registered agent boundary OK");
