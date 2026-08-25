import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const labConfig = JSON.parse(
  readFileSync(
    path.resolve(__dirname, "../../src-tauri/tauri.codex-lab.conf.json"),
    "utf8",
  ),
);

test("Codex Lab installer registers upstream and isolated Buzz deep-link schemes", () => {
  const schemes = labConfig.plugins?.["deep-link"]?.desktop?.schemes ?? [];
  assert.ok(schemes.includes("buzz"));
  assert.ok(schemes.includes("buzz-codex-lab"));
});
