import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  changedWorkspaceCrates,
  commandsForWorkspaceCrate,
  parsePackageName,
  parseWorkspaceMembers,
  selectTauriPlan,
  selectWorkspacePlan,
} from "./pre-push-rust.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT_PATH = resolve(REPO_ROOT, "scripts/pre-push-rust.mjs");
const scratch = mkdtempSync(join(tmpdir(), "buzz-pre-push-rust-test-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

test("parseWorkspaceMembers and parsePackageName handle real Cargo.toml files", () => {
  const rootCargo = readFileSync(join(REPO_ROOT, "Cargo.toml"), "utf8");
  const members = parseWorkspaceMembers(rootCargo);
  assert.ok(members.includes("crates/buzz-db"));
  assert.ok(members.includes("crates/buzz-relay"));
  assert.ok(members.includes("examples/countdown-bot"));
  assert.ok(!members.includes("desktop/src-tauri"));

  // buzz-pair-relay has [package] name = "buzz-pair-relay" and [lib] name = "buzz_pair_relay"
  const pairRelayToml = readFileSync(
    join(REPO_ROOT, "crates/buzz-pair-relay/Cargo.toml"),
    "utf8",
  );
  assert.equal(parsePackageName(pairRelayToml), "buzz-pair-relay");
});

test("changedWorkspaceCrates maps changed paths without reverse dependencies", () => {
  const crates = changedWorkspaceCrates(
    [
      "crates/buzz-db/src/runtime/tests.rs",
      "crates/buzz-db/src/store/event.rs",
      "Cargo.lock",
      "migrations/0001_init.sql",
    ],
    REPO_ROOT,
  );
  assert.deepEqual(crates, [{ name: "buzz-db", dir: "crates/buzz-db" }]);
});

test("changedWorkspaceCrates returns empty when only config, lockfiles, migrations, or non-Rust files changed", () => {
  const crates = changedWorkspaceCrates(
    [
      "Cargo.toml",
      "Cargo.lock",
      "rust-toolchain.toml",
      "deny.toml",
      "migrations/0002_foo.sql",
      "schema/tables/public/events.sql",
      "Justfile",
      "scripts/run-tests.sh",
      "desktop/src/App.tsx",
      "docs/contributing.md",
    ],
    REPO_ROOT,
  );
  assert.deepEqual(crates, []);
});

test("changedWorkspaceCrates skips deleted crates and includes examples/countdown-bot", () => {
  const crates = changedWorkspaceCrates(
    ["crates/does-not-exist/src/lib.rs", "examples/countdown-bot/src/main.rs"],
    REPO_ROOT,
  );
  assert.deepEqual(crates, [
    { name: "countdown-bot", dir: "examples/countdown-bot" },
  ]);
});

test("selectWorkspacePlan keeps infra-free filters and features per crate", () => {
  const dbPlan = selectWorkspacePlan({
    files: ["crates/buzz-db/src/store/event.rs"],
    repoRoot: REPO_ROOT,
    hasNextest: true,
  });
  assert.deepEqual(
    dbPlan.commands.map((c) => c.args),
    [
      ["nextest", "run", "-p", "buzz-db", "--lib"],
      ["nextest", "run", "-p", "buzz-db", "--test", "observability_source"],
      ["test", "-p", "buzz-db", "--doc"],
    ],
  );

  const ffPlan = selectWorkspacePlan({
    files: ["crates/buzz-feature-flags/src/lib.rs"],
    repoRoot: REPO_ROOT,
    hasNextest: true,
  });
  assert.deepEqual(
    ffPlan.commands.map((c) => c.args),
    [
      [
        "nextest",
        "run",
        "-p",
        "buzz-feature-flags",
        "--features",
        "launchdarkly",
      ],
    ],
  );

  const pgPlan = selectWorkspacePlan({
    files: ["crates/buzz-push-gateway/src/main.rs"],
    repoRoot: REPO_ROOT,
    hasNextest: true,
  });
  assert.deepEqual(
    pgPlan.commands.map((c) => c.args),
    [
      ["nextest", "run", "-p", "buzz-push-gateway"],
      [
        "nextest",
        "run",
        "-p",
        "buzz-push-gateway",
        "--features",
        "personal-dev-app-attest",
      ],
    ],
  );

  const relayPlan = selectWorkspacePlan({
    files: ["crates/buzz-relay/src/lib.rs"],
    repoRoot: REPO_ROOT,
    hasNextest: true,
  });
  assert.ok(relayPlan.commands.length >= 8);
  for (const step of relayPlan.commands) {
    assert.ok(
      step.args.includes("-E") || step.args.includes("boot_lifecycle"),
      `buzz-relay must stay filtered to infra-free tests: ${step.args.join(" ")}`,
    );
  }
});

test("selectWorkspacePlan covers every crate in Justfile test-unit without drift", () => {
  const justfile = readFileSync(join(REPO_ROOT, "Justfile"), "utf8");
  const testUnitSection = justfile.match(
    /^test-unit:\n([\s\S]*?)^\n# Run integration tests/m,
  )?.[1];
  assert.ok(testUnitSection, "Could not locate test-unit recipe in Justfile");

  const nextestCrates = new Set();
  for (const [, pkg] of testUnitSection.matchAll(
    /cargo nextest run -p ([\w-]+)/g,
  )) {
    nextestCrates.add(pkg);
  }
  for (const [, pkg] of testUnitSection.matchAll(/cargo test -p ([\w-]+)/g)) {
    nextestCrates.add(pkg);
  }

  for (const pkg of nextestCrates) {
    const nextestCmds = commandsForWorkspaceCrate(pkg, { hasNextest: true });
    const cargoCmds = commandsForWorkspaceCrate(pkg, { hasNextest: false });
    assert.ok(nextestCmds.length > 0, `Missing nextest commands for ${pkg}`);
    assert.ok(
      cargoCmds.length > 0,
      `Missing cargo fallback commands for ${pkg}`,
    );
  }

  // Every exact test(=...) selector in Justfile's test-unit buzz-relay block must
  // remain present in the pre-push buzz-relay plan.
  const relayCmdText = commandsForWorkspaceCrate("buzz-relay", {
    hasNextest: true,
  })
    .map((c) => c.args.join(" "))
    .join("\n");
  for (const [, exactTest] of testUnitSection.matchAll(/test\(=([^)]+)\)/g)) {
    if (
      exactTest ===
      "tests::queue_full_retain_does_not_wait_and_schedules_reconciliation"
    )
      continue;
    if (
      exactTest ===
      "bucket_index::tests::bucket_snapshot_json_round_trip_preserves_community_keys"
    )
      continue;
    if (
      exactTest ===
      "tests::partition_audit_outcomes_keep_their_json_names_and_exit_codes"
    )
      continue;
    assert.ok(
      relayCmdText.includes(`test(=${exactTest})`),
      `pre-push buzz-relay filter is missing Justfile test-unit case: ${exactTest}`,
    );
  }
});

test("selectTauriPlan ignores non-Tauri and lockfile-only changes, and scopes buzz-terminal vs buzz-desktop", () => {
  assert.equal(
    selectTauriPlan({
      files: ["crates/buzz-db/src/lib.rs", "desktop/src/App.tsx"],
      repoRoot: REPO_ROOT,
    }).scope,
    "none",
  );

  assert.equal(
    selectTauriPlan({
      files: ["desktop/src-tauri/Cargo.lock", "desktop/src-tauri/README.md"],
      repoRoot: REPO_ROOT,
    }).scope,
    "none",
  );

  const termPlan = selectTauriPlan({
    files: ["desktop/src-tauri/crates/buzz-terminal/src/lib.rs"],
    repoRoot: REPO_ROOT,
  });
  assert.equal(termPlan.scope, "buzz-terminal");
  assert.deepEqual(
    termPlan.commands.map((c) => [c.cmd, ...c.args]),
    [
      [
        "cargo",
        "clippy",
        "--manifest-path",
        "desktop/src-tauri/Cargo.toml",
        "-p",
        "buzz-terminal",
        "--all-targets",
        "--",
        "-D",
        "warnings",
      ],
      [
        "cargo",
        "test",
        "--manifest-path",
        "desktop/src-tauri/Cargo.toml",
        "-p",
        "buzz-terminal",
      ],
    ],
  );

  const desktopPlan = selectTauriPlan({
    files: ["desktop/src-tauri/src/commands/updater.rs"],
    repoRoot: REPO_ROOT,
  });
  assert.equal(desktopPlan.scope, "buzz-desktop");
  assert.deepEqual(
    desktopPlan.commands.map((c) => [c.cmd, ...c.args]),
    [
      ["just", "_ensure-sidecar-stubs"],
      [
        "cargo",
        "clippy",
        "--manifest-path",
        "desktop/src-tauri/Cargo.toml",
        "-p",
        "buzz-desktop",
        "--all-targets",
        "--",
        "-D",
        "warnings",
      ],
      [
        "cargo",
        "clippy",
        "--manifest-path",
        "desktop/src-tauri/Cargo.toml",
        "-p",
        "buzz-desktop",
        "--all-targets",
        "--features",
        "mesh-llm",
        "--",
        "-D",
        "warnings",
      ],
      ["cargo", "test", "-p", "buzz-desktop"],
    ],
  );

  const workspacePlan = selectTauriPlan({
    files: [
      "desktop/src-tauri/Cargo.toml",
      "desktop/src-tauri/crates/buzz-terminal/src/lib.rs",
    ],
    repoRoot: REPO_ROOT,
  });
  assert.equal(workspacePlan.scope, "workspace");
  assert.deepEqual(
    workspacePlan.commands.map((c) => [c.cmd, ...c.args]),
    [
      ["just", "desktop-tauri-clippy"],
      ["just", "desktop-tauri-test"],
    ],
  );
});

function createGitFixture(name) {
  const dir = mkdtempSync(join(scratch, `${name}-`));
  const emptyHooks = join(dir, ".empty-hooks");
  mkdirSync(emptyHooks, { recursive: true });
  const git = (...args) =>
    execFileSync("git", args, { cwd: dir, encoding: "utf8" }).trim();
  git("init", "-q");
  git("config", "core.hooksPath", emptyHooks);
  git("config", "user.name", "Test");
  git("config", "user.email", "test@example.com");
  git("config", "commit.gpgsign", "false");

  const writeRel = (rel, content) => {
    const abs = join(dir, rel);
    mkdirSync(dirname(abs), { recursive: true });
    writeFileSync(abs, content);
  };

  writeRel(
    "Cargo.toml",
    '[workspace]\nmembers = ["crates/alpha", "crates/beta"]\n',
  );
  writeRel("crates/alpha/Cargo.toml", '[package]\nname = "alpha"\n');
  writeRel("crates/alpha/src/lib.rs", "pub fn a() {}\n");
  writeRel("crates/beta/Cargo.toml", '[package]\nname = "beta"\n');
  writeRel("crates/beta/src/lib.rs", "pub fn b() {}\n");
  writeRel(
    "desktop/src-tauri/Cargo.toml",
    '[workspace]\nmembers = ["crates/buzz-terminal"]\n[package]\nname = "buzz-desktop"\n',
  );
  writeRel(
    "desktop/src-tauri/crates/buzz-terminal/Cargo.toml",
    '[package]\nname = "buzz-terminal"\n',
  );
  writeRel("Cargo.lock", "# lock\n");
  writeRel("migrations/0001_init.sql", "-- init\n");
  git("add", ".");
  git("commit", "-qm", "initial");

  return { dir, git, writeRel };
}

test("CLI skips safely when origin/main is absent and runs only changed crates with stub runner", () => {
  const { dir, git, writeRel } = createGitFixture("cli");
  const stubBin = join(dir, ".stub-bin");
  mkdirSync(stubBin, { recursive: true });
  const logFile = join(dir, "invocations.log");
  for (const binName of ["cargo", "cargo-nextest", "just"]) {
    const binPath = join(stubBin, binName);
    writeFileSync(
      binPath,
      binName === "cargo-nextest"
        ? "#!/bin/sh\nexit 0\n"
        : `#!/bin/sh\necho "${binName} $*" >> "${logFile}"\nexit 0\n`,
    );
    chmodSync(binPath, 0o755);
  }
  const env = { ...process.env, PATH: `${stubBin}:${process.env.PATH}` };

  // No origin/main: must skip instead of running full suite.
  const noBase = spawnSync(
    process.execPath,
    [SCRIPT_PATH, "workspace", "--root", dir],
    { cwd: dir, env, encoding: "utf8" },
  );
  assert.equal(noBase.status, 0, noBase.stderr);
  assert.match(noBase.stdout, /No local origin\/main; skipping/);

  // Create origin/main at base, commit only Cargo.lock + migrations on branch.
  git("update-ref", "refs/remotes/origin/main", "HEAD");
  writeRel("Cargo.lock", "# updated\n");
  writeRel("migrations/0002_next.sql", "-- next\n");
  git("add", ".");
  git("commit", "-qm", "lock and migration only");

  const lockOnly = spawnSync(
    process.execPath,
    [SCRIPT_PATH, "workspace", "--root", dir],
    { cwd: dir, env, encoding: "utf8" },
  );
  assert.equal(lockOnly.status, 0, lockOnly.stderr);
  assert.match(lockOnly.stdout, /No workspace crate changed/);

  // Now also touch crates/alpha: must run ONLY alpha, never beta.
  writeRel("crates/alpha/src/lib.rs", "pub fn a() -> i32 { 1 }\n");
  git("add", ".");
  git("commit", "-qm", "touch alpha");

  const alphaOnly = spawnSync(
    process.execPath,
    [SCRIPT_PATH, "workspace", "--root", dir],
    { cwd: dir, env, encoding: "utf8" },
  );
  assert.equal(alphaOnly.status, 0, alphaOnly.stderr);
  const invocations = readFileSync(logFile, "utf8").trim().split("\n");
  assert.deepEqual(invocations, ["cargo nextest run -p alpha"]);

  // Failing stub command propagates non-zero exit code.
  writeFileSync(join(stubBin, "cargo"), "#!/bin/sh\nexit 7\n");
  const failing = spawnSync(
    process.execPath,
    [SCRIPT_PATH, "workspace", "--root", dir],
    { cwd: dir, env, encoding: "utf8" },
  );
  assert.equal(failing.status, 7);
});
