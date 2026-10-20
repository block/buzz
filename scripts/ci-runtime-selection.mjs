import assert from "node:assert/strict";
import { appendFileSync, readFileSync } from "node:fs";

const filters = JSON.parse(process.env.FILTER_OUTPUTS);
let runAll = false;
if (process.env.GITHUB_EVENT_NAME === "pull_request") {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
  const count = event.pull_request.changed_files;
  assert.ok(
    Number.isSafeInteger(count) && count >= 0,
    "Invalid PR changed_files",
  );
  // GitHub's PR-files endpoint returns at most 3,000 files. Never use a
  // potentially incomplete list to skip a runtime suite.
  runAll = count >= 3000;
}
for (const key of ["rust", "desktop", "desktop-rust", "web", "mobile"]) {
  assert.ok(
    ["true", "false"].includes(filters[key]),
    `Invalid ${key} selection`,
  );
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    `${key}=${runAll ? "true" : filters[key]}\n`,
  );
}
