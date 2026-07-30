#!/usr/bin/env node
import fs from "node:fs";
import { evaluateSemanticHealth } from "./worker.mjs";

const source = process.argv[2] ? fs.readFileSync(process.argv[2], "utf8") : fs.readFileSync(0, "utf8");
const evidence = JSON.parse(source);
const checks = (Array.isArray(evidence) ? evidence : [evidence]).map((entry) => ({
  aspect: entry.aspect,
  ...evaluateSemanticHealth(entry),
}));
const result = {
  schema: "aeon_buzz_semantic_health_v1",
  ok: checks.length > 0 && checks.every((check) => check.healthy),
  checks,
};

process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
if (!result.ok) process.exitCode = 1;
