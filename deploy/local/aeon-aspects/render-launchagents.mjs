#!/usr/bin/env node
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadJson, renderDisabledLaunchAgent, validateManifest } from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifest = loadJson(join(here, "workers.json"));
const identityMap = loadJson(process.argv[2] ?? join(here, "fixtures", "identity-map.json"));
const validation = validateManifest(manifest, identityMap);
if (!validation.ok) {
  console.error(validation.errors.join("\n"));
  process.exit(1);
}
const artifacts = Object.fromEntries(
  manifest.workers.map((worker) => {
    const artifact = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    return [`${artifact.label}.plist`, artifact.plist];
  }),
);
process.stdout.write(`${JSON.stringify({ schema: "aeon_disabled_launchagents_v1", artifacts }, null, 2)}\n`);
