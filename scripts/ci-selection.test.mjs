import assert from "node:assert/strict";
import { execFile, execFileSync } from "node:child_process";
import {
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import { promisify } from "node:util";

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
async function select(paths, pullRequest = false) {
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
  let server;
  let apiUrl;
  const eventPath = join(scratch, `event-${repo.split("/").pop()}.json`);
  if (pullRequest) {
    const commit = (message) =>
      git(
        "-c",
        "user.name=CI fixture",
        "-c",
        "user.email=ci@example.com",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        message,
        "-s",
      );
    const base = git("rev-parse", "HEAD").toString().trim();
    commit("PR documentation or code");
    const head = git("rev-parse", "HEAD").toString().trim();
    git("checkout", "-qb", "upstream", base);
    const upstreamPath = "desktop/src/upstream-only.ts";
    mkdirSync(dirname(join(repo, upstreamPath)), { recursive: true });
    writeFileSync(join(repo, upstreamPath), "upstream change\n");
    git("add", upstreamPath);
    commit("Unrelated upstream desktop change");
    git(
      "-c",
      "user.name=CI fixture",
      "-c",
      "user.email=ci@example.com",
      "-c",
      "commit.gpgsign=false",
      "merge",
      "--no-ff",
      "-m",
      "Synthetic merge",
      head,
    );
    git("checkout", "--detach");
    writeFileSync(
      eventPath,
      JSON.stringify({
        pull_request: {
          number: 7809,
          base: { sha: base },
          head: { sha: head },
        },
        repository: { default_branch: "main" },
      }),
    );
    // Serve the PR file list, which intentionally excludes the newer base's code.
    server = createServer((request, response) => {
      assert.equal(
        request.url,
        "/repos/block/buzz/pulls/7809/files?per_page=100",
      );
      response.setHeader("Content-Type", "application/json");
      response.end(
        JSON.stringify(
          paths.map((filename) => ({ filename, status: "added" })),
        ),
      );
    });
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    apiUrl = `http://127.0.0.1:${server.address().port}`;
  }
  const output = join(repo, "action-output");
  writeFileSync(output, "");
  try {
    await promisify(execFile)(process.execPath, [actionPath], {
      cwd: repo,
      encoding: "utf8",
      timeout: 30000,
      env: {
        ...process.env,
        INPUT_BASE: pullRequest ? "" : "HEAD",
        INPUT_FILTERS: filters,
        // Resolve the workflow's token expression to a fixture token. Removing
        // it must reproduce the contaminated git comparison in the PR fixture.
        INPUT_TOKEN:
          pullRequest && /token: \$\{\{ github\.token \}\}/.test(workflow)
            ? "fixture-token"
            : "",
        INPUT_REF: "",
        GITHUB_EVENT_NAME: pullRequest ? "pull_request" : "",
        GITHUB_EVENT_PATH: pullRequest ? eventPath : "",
        GITHUB_REPOSITORY: "block/buzz",
        GITHUB_API_URL: apiUrl || "https://api.github.com",
        GITHUB_OUTPUT: output,
        "INPUT_PREDICATE-QUANTIFIER":
          workflow.match(/predicate-quantifier: ['"]?([\w-]+)/)?.[1] ?? "some",
      },
    });
  } finally {
    if (server) await new Promise((resolve) => server.close(resolve));
  }
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
  ["integration services", ["docker-compose.yml"], ["rust"]],
  ["CI integration images", ["docker-compose.ci.yml"], ["rust"]],
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
  [
    "incident documentation",
    ["CONTEXT.md", "docs/mobile-push-suppression.md", "VISION_MOBILE.md"],
    [],
  ],
  [
    "nested documentation",
    [
      "crates/buzz-cli/README.md",
      "desktop/README.md",
      "desktop/src-tauri/README.md",
      "web/docs/design.md",
      "mobile/test/README.md",
      "schema/README.md",
      "migrations/README.md",
    ],
    [],
  ],
  [
    "mixed documentation and desktop",
    ["VISION_MOBILE.md", "mobile/README.md", "desktop/src/main.tsx"],
    ["desktop"],
  ],
  [
    "mixed documentation and relay",
    ["desktop/README.md", "crates/buzz-relay/src/lib.rs"],
    ["rust"],
  ],
  ["embedded ACP prompt", ["crates/buzz-acp/src/base_prompt.md"], ["rust"]],
  [
    "embedded Tauri skill",
    ["desktop/src-tauri/src/managed_agents/nest_skill.md"],
    ["desktop", "desktop-rust"],
  ],
];
for (const [name, paths, expected] of scenarios) {
  test(`real paths-filter: ${name}`, async () => {
    const outputs = await select(paths);
    assert.equal(Object.keys(outputs).length, 5);
    assert.deepEqual(
      Object.keys(outputs)
        .filter((key) => outputs[key] === "true")
        .sort(),
      expected.sort(),
    );
  });
}

for (const [name, paths, expected] of [
  [
    "docs-only",
    ["CONTEXT.md", "docs/mobile-push-suppression.md", "VISION_MOBILE.md"],
    [],
  ],
  [
    "mixed mobile and Markdown",
    ["VISION_MOBILE.md", "mobile/lib/main.dart"],
    ["mobile"],
  ],
  [
    "mixed desktop and Markdown",
    ["CONTEXT.md", "desktop/src/main.tsx"],
    ["desktop"],
  ],
]) {
  test(`PR file list ignores newer base changes: ${name}`, async () => {
    const outputs = await select(paths, true);
    assert.equal(Object.keys(outputs).length, 5);
    assert.deepEqual(
      Object.keys(outputs)
        .filter((key) => outputs[key] === "true")
        .sort(),
      expected.sort(),
    );
  });
}
