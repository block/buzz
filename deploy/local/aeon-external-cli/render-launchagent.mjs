#!/usr/bin/env node
import fs from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  loadJson,
  renderDisabledLaunchAgent,
  validateManifest,
  validateSubscriptionProjection,
} from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
function option(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

const workspace = option("--workspace");
const identityPath = option("--identity-map") ?? join(here, "fixtures", "identity-map.json");
const worker = option("--worker") ?? "codex_cli";
const manifestName = worker === "codex_cli" ? "manifest.json" : `manifest.${worker}.json`;
if (!["codex_cli", "claude_cli", "cursor_cli", "grok_cli"].includes(worker)) {
  console.error(`unsupported external CLI worker: ${worker}`);
  process.exit(1);
}
const manifest = loadJson(join(here, manifestName));
const identityMap = loadJson(identityPath);
const validation = validateManifest(manifest, identityMap);
if (!validation.ok) {
  console.error(validation.errors.join("\n"));
  process.exit(1);
}
const selector = manifest.worker.selector ?? manifest.worker.principal;
const configText = fs.readFileSync(join(here, "config", `${selector}.toml`), "utf8");
const subscriptionValidation = validateSubscriptionProjection(configText, manifest, identityMap);
if (!subscriptionValidation.ok) {
  console.error(subscriptionValidation.errors.join("\n"));
  process.exit(1);
}

process.stdout.write(renderDisabledLaunchAgent(manifest, identityMap, workspace).plist);
