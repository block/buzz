#!/usr/bin/env node
import fs from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadJson, renderPrivateOfficePrompt, validateManifest } from "./worker.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const manifest = loadJson(join(here, "workers.json"));
const identityMap = loadJson(process.argv[2] ?? join(here, "fixtures", "identity-map.json"));
const validation = validateManifest(manifest, identityMap);
if (!validation.ok) {
  console.error(validation.errors.join("\n"));
  process.exit(1);
}
const template = fs.readFileSync(join(here, "prompts", "private-office.template.md"), "utf8");
const artifacts = Object.fromEntries(
  manifest.workers.map((worker) => [
    `${worker.aspect}-private-office.md`,
    renderPrivateOfficePrompt(template, worker.aspect),
  ]),
);
process.stdout.write(
  `${JSON.stringify({ schema: "aeon_private_office_prompts_v1", artifacts }, null, 2)}\n`,
);
