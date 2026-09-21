import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";

const workflow = readFileSync(
  new URL("../.github/workflows/ci.yml", import.meta.url),
  "utf8",
);
const scratch = mkdtempSync(join(tmpdir(), "buzz-ci-selection-"));
after(() => rmSync(scratch, { recursive: true, force: true }));
const actionSha = workflow.match(/uses: dorny\/paths-filter@([a-f0-9]{40})/)[1];
const filters = workflow
  .match(/ {10}filters: \|\n([\s\S]*?)(?= {6}- name:)/)[1]
  .split("\n")
  .map((line) => line.slice(12))
  .join("\n");
// GitHub-hosted runners prepare pinned actions beside RUNNER_TEMP before any
// steps run. Reuse that bundle, without another download in the selection gate.
// Locally, point PATHS_FILTER_ACTION at dist/index.js from the pinned action.
const actionPath =
  process.env.PATHS_FILTER_ACTION ||
  (process.env.RUNNER_TEMP &&
    join(
      process.env.RUNNER_TEMP,
      "..",
      "_actions",
      "dorny",
      "paths-filter",
      actionSha,
      "dist",
      "index.js",
    ));
assert.ok(
  actionPath && existsSync(actionPath),
  `Set PATHS_FILTER_ACTION to the local dist/index.js from dorny/paths-filter@${actionSha}`,
);
function select(paths) {
  const repo = mkdtempSync(join(scratch, "repo-"));
  const git = (...args) =>
    execFileSync("git", args, { cwd: repo, stdio: "pipe", timeout: 10000 });
  git("init", "-q");
  // Fixture repositories must not inherit developer machine hooks.
  mkdirSync(join(repo, "empty-hooks"));
  git("config", "core.hooksPath", join(repo, "empty-hooks"));
  git(
    "-c",
    "user.name=CI fixture",
    "-c",
    "user.email=ci@example.com",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "-q",
    "--allow-empty",
    "-m",
    "fixture",
    "-s",
  );
  for (const path of paths) {
    mkdirSync(dirname(join(repo, path)), { recursive: true });
    writeFileSync(join(repo, path), "fixture\n");
  }
  git("add", ".");
  const output = join(repo, "action-output");
  writeFileSync(output, "");
  const child = spawnSync(process.execPath, [actionPath], {
    cwd: repo,
    encoding: "utf8",
    timeout: 30000,
    env: {
      ...process.env,
      INPUT_BASE: "HEAD",
      INPUT_FILTERS: filters,
      INPUT_TOKEN: "",
      INPUT_REF: "",
      GITHUB_OUTPUT: output,
      "INPUT_PREDICATE-QUANTIFIER":
        workflow.match(/predicate-quantifier: ['"]?([\w-]+)/)?.[1] ?? "some",
    },
  });
  assert.equal(child.status, 0, child.stdout + child.stderr);
  return Object.fromEntries(
    [
      ...readFileSync(output, "utf8").matchAll(
        /^(rust|desktop|desktop-rust|web|mobile)<<([^\n]+)\n(true|false)\n\2/gm,
      ),
    ].map(([, key, , value]) => [key, value]),
  );
}
const scenarios = [
  [
    "mobile source and tests",
    [
      "mobile/lib/features/age_gate/age_signal_provider.dart",
      "mobile/test/features/age_gate/age_signal_provider_test.dart",
    ],
    ["mobile"],
  ],
  ["mobile lockfile", ["mobile/pubspec.lock"], ["mobile"]],
  ["mobile release script", ["scripts/mobile-release.sh"], ["mobile"]],
  ["desktop", ["desktop/src/main.tsx"], ["desktop"]],
  ["Tauri", ["desktop/src-tauri/src/main.rs"], ["desktop", "desktop-rust"]],
  ["relay", ["crates/buzz-relay/src/main.rs"], ["rust"]],
  ["migration", ["migrations/123.sql"], ["rust"]],
  ["shared workflow", [".github/workflows/ci.yml"], ["rust", "mobile"]],
  [
    "mixed mobile and relay",
    ["mobile/lib/main.dart", "crates/buzz-core/src/lib.rs"],
    ["rust", "mobile"],
  ],
  [
    "mixed mobile and desktop",
    ["mobile/lib/main.dart", "desktop/src/main.tsx"],
    ["desktop", "mobile"],
  ],
  ["web", ["web/src/main.tsx"], ["web"]],
  ["documentation", ["README.md"], []],
];
for (const [name, paths, expected] of scenarios) {
  test(`real paths-filter: ${name}`, () => {
    const outputs = select(paths);
    assert.equal(Object.keys(outputs).length, 5);
    assert.deepEqual(
      Object.keys(outputs)
        .filter((key) => outputs[key] === "true")
        .sort(),
      expected.sort(),
    );
  });
}
