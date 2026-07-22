#!/usr/bin/env node
import fs from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadJson, renderDisabledLaunchAgent, renderWorker, validateManifest } from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifestPath = join(here, "workers.json");
const manifest = loadJson(manifestPath);
const identityMap = loadJson(process.argv[2] ?? manifest.identityMap);
const validation = validateManifest(manifest, identityMap);
const rendered = validation.ok ? manifest.workers.map((worker) => renderWorker(manifest, identityMap, worker.aspect)) : [];
const launchAgents = [];
if (validation.ok) {
  for (const worker of manifest.workers) {
    const artifact = renderDisabledLaunchAgent(manifest, identityMap, worker.aspect);
    const path = join(here, "launchagents", `${artifact.label}.plist`);
    if (!fs.existsSync(path) || fs.readFileSync(path, "utf8") !== artifact.plist) {
      validation.errors.push(`${worker.aspect}: checked-in LaunchAgent preview drift`);
    }
    launchAgents.push({ label: artifact.label, path, runAtLoad: artifact.runAtLoad, keepAlive: artifact.keepAlive });
  }
  validation.ok = validation.errors.length === 0;
}
console.log(JSON.stringify({ ...validation, workers: rendered, launchAgents }, null, 2));
process.exitCode = validation.ok ? 0 : 1;
