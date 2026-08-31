import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { rules as desktopRules } from "../desktop/scripts/file-size-rules.mjs";
import { rules as webRules } from "../web/scripts/file-size-rules.mjs";
import {
  allowedLineCount,
  countLines,
  evaluateFileSize,
  parseChangedFiles,
  resolveBaseRef,
} from "./check-file-sizes-core.mjs";

function git(repo, ...args) {
  // These fixture repositories inherit both hook configuration and Git's
  // repository-local environment when this test runs from pre-push. Isolate
  // them completely so fixture commits cannot recurse into the real checkout.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")),
  );
  return execFileSync("git", ["-c", "core.hooksPath=/dev/null", ...args], {
    cwd: repo,
    encoding: "utf8",
    env,
  }).trim();
}

test("local base resolution uses the branch merge-base and fails without origin/main", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "file-size-base-"));
  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  git(repo, "commit", "--allow-empty", "-m", "base");
  git(repo, "remote", "add", "origin", repo);
  git(repo, "fetch", "origin", "main:refs/remotes/origin/main");
  const base = git(repo, "rev-parse", "HEAD");
  git(repo, "switch", "-c", "feature");
  git(repo, "commit", "--allow-empty", "-m", "first branch commit");
  git(repo, "commit", "--allow-empty", "-m", "second branch commit");

  assert.equal(resolveBaseRef(repo, {}), base);
  git(repo, "update-ref", "-d", "refs/remotes/origin/main");
  assert.throws(
    () => resolveBaseRef(repo, {}),
    /Fetch origin\/main or set CHECK_FILE_SIZES_BASE/,
  );
});

test("counts empty, LF, and CRLF content with the existing semantics", () => {
  assert.equal(countLines(""), 0);
  assert.equal(countLines("one\n"), 2);
  assert.equal(countLines("one\r\ntwo"), 2);
});

test("new files use the configured ceiling", () => {
  assert.equal(allowedLineCount(null, 1000), 1000);
  assert.deepEqual(
    evaluateFileSize({ baseLines: null, candidateLines: 1000, maxLines: 1000 }),
    {
      limit: 1000,
      violates: false,
    },
  );
  assert.equal(
    evaluateFileSize({ baseLines: null, candidateLines: 1001, maxLines: 1000 })
      .violates,
    true,
  );
});

test("a compliant file may not cross the ceiling", () => {
  assert.equal(
    evaluateFileSize({ baseLines: 996, candidateLines: 1000, maxLines: 1000 })
      .violates,
    false,
  );
  assert.equal(
    evaluateFileSize({ baseLines: 996, candidateLines: 1003, maxLines: 1000 })
      .violates,
    true,
  );
});

test("parses modifications, deletions, and renames from Git's NUL format", () => {
  assert.deepEqual(
    parseChangedFiles(
      "M\0desktop/src/a.ts\0D\0desktop/src/b.ts\0R100\0desktop/src/old.ts\0desktop/src/new.ts\0",
    ),
    [
      { status: "M", path: "desktop/src/a.ts" },
      { status: "D", path: "desktop/src/b.ts" },
      {
        status: "R",
        oldPath: "desktop/src/old.ts",
        path: "desktop/src/new.ts",
      },
    ],
  );
});

test("an inherited oversized file may hold or shrink but not grow", () => {
  assert.equal(allowedLineCount(1026, 1000), 1026);
  assert.equal(
    evaluateFileSize({ baseLines: 1026, candidateLines: 1026, maxLines: 1000 })
      .violates,
    false,
  );
  assert.equal(
    evaluateFileSize({ baseLines: 1026, candidateLines: 1001, maxLines: 1000 })
      .violates,
    false,
  );
  assert.equal(
    evaluateFileSize({ baseLines: 1026, candidateLines: 1027, maxLines: 1000 })
      .violates,
    true,
  );
});

// --- The gate's own coverage ---------------------------------------------
//
// An omitted extension does not fail loudly: `runFileSizeCheck` skips the file
// and the check still exits 0, so an uncovered root looks exactly like a clean
// one. That makes the allowlist something to assert rather than assume. These
// read the real rule tables the runners execute, not a restatement of them.

const scriptsDir = path.dirname(fileURLToPath(import.meta.url));

test("every script root governs .mjs alongside .ts and .tsx", () => {
  for (const [label, rules] of [
    ["desktop", desktopRules],
    ["web", webRules],
  ]) {
    const scriptRoots = rules.filter((rule) => rule.extensions.has(".ts"));
    assert.ok(
      scriptRoots.length > 0,
      `${label} declares no TypeScript roots, so this assertion cannot fail for the reason it exists; the rule table shape changed`,
    );
    for (const rule of scriptRoots) {
      assert.ok(
        rule.extensions.has(".mjs"),
        `${label} root ${rule.root} governs .ts but not .mjs, so test modules and shared rigs there sit outside the ${rule.maxLines}-line ceiling`,
      );
    }
  }
});

// Runs the real desktop rule table against a throwaway repository in a child
// process. A child keeps `process.exitCode` and `console.error` out of this
// test's own process, where a leaked exit code would mark the whole file failed.
const COMPLETION_SENTINEL = "__file_size_gate_completed__";

function runDesktopGate({ fixtureRoot, baseRef }) {
  const result = spawnSync(
    process.execPath,
    [
      "--input-type=module",
      "-e",
      `
      import { runFileSizeCheck } from ${JSON.stringify(path.join(scriptsDir, "check-file-sizes-core.mjs"))};
      import { rules } from ${JSON.stringify(path.join(scriptsDir, "..", "desktop", "scripts", "file-size-rules.mjs"))};
      await runFileSizeCheck({
        projectRoot: ${JSON.stringify(path.join(fixtureRoot, "desktop"))},
        rules,
        label: "Desktop",
      });
      // Printed only if the awaited call returned normally. On stdout so it
      // cannot be confused with any part of the violation report.
      process.stdout.write(${JSON.stringify(COMPLETION_SENTINEL)});
      `,
    ],
    {
      encoding: "utf8",
      env: {
        ...Object.fromEntries(
          Object.entries(process.env).filter(
            ([key]) => !key.startsWith("GIT_"),
          ),
        ),
        CHECK_FILE_SIZES_BASE: baseRef,
      },
    },
  );
  // Node exits 1 for an uncaught exception too, so status alone cannot tell a
  // ratchet violation from a crash. Split the two questions: the sentinel proves
  // the gate ran to completion, and only then does the exit status mean a policy
  // decision. Asserting on the report heading alone would accept a child that
  // started printing violations and then crashed part-way through.
  assert.ok(
    result.stdout.includes(COMPLETION_SENTINEL),
    `the ratchet child did not run to completion (exit ${result.status}, signal ${result.signal}), so it crashed rather than gated:\n${result.stderr}`,
  );
  const failed = result.status === 1;
  if (failed) {
    assert.match(
      result.stderr,
      /file size ratchet failed \(base /,
      `the ratchet child exited 1 without reporting a violation:\n${result.stderr}`,
    );
  } else {
    assert.equal(
      result.status,
      0,
      `the ratchet child exited ${result.status} (signal ${result.signal}):\n${result.stderr}`,
    );
  }
  return { failed, output: result.stderr };
}

function fixtureRepo(prefix) {
  const repo = mkdtempSync(path.join(tmpdir(), prefix));
  mkdirSync(path.join(repo, "desktop/src/features/agents"), {
    recursive: true,
  });
  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  return repo;
}

test("the desktop rules hold a new .mjs file in a governed root to the ceiling", () => {
  const repo = fixtureRepo("file-size-mjs-new-");
  git(repo, "commit", "--allow-empty", "-m", "base");
  const target = "desktop/src/features/agents/oversize.test.mjs";

  // `countLines` counts a trailing newline as a final empty line, so N repeats
  // of a newline-terminated line is N + 1 lines. Land exactly on the ceiling.
  writeFileSync(path.join(repo, target), `${"// line\n".repeat(999)}// line`);
  const atCeiling = runDesktopGate({ fixtureRoot: repo, baseRef: "HEAD" });
  assert.equal(
    atCeiling.failed,
    false,
    `a new .mjs at exactly the ceiling must pass: ${atCeiling.output}`,
  );

  writeFileSync(path.join(repo, target), "// line\n".repeat(1000));
  const overCeiling = runDesktopGate({ fixtureRoot: repo, baseRef: "HEAD" });
  assert.equal(
    overCeiling.failed,
    true,
    "a new 1001-line .mjs under src/features must violate the ceiling",
  );
  assert.match(
    overCeiling.output,
    /src\/features\/agents\/oversize\.test\.mjs: new -> 1001 lines \(allowed 1000\)/,
  );
});

test("an inherited oversize .mjs holds or shrinks but may not grow", () => {
  const repo = fixtureRepo("file-size-mjs-inherited-");
  const target = "desktop/src/features/agents/inherited.test.mjs";

  // Commit it already over the ceiling, as the existing oversize suites are.
  writeFileSync(path.join(repo, target), "// line\n".repeat(1199));
  git(repo, "add", "-A");
  git(repo, "commit", "-m", "inherited oversize test module");
  const baseRef = git(repo, "rev-parse", "HEAD");

  // Same line count, different content: an edit that does not grow the file.
  writeFileSync(path.join(repo, target), "// edit\n".repeat(1199));
  const held = runDesktopGate({ fixtureRoot: repo, baseRef });
  assert.equal(
    held.failed,
    false,
    `an inherited oversize .mjs must be grandfathered, not failed on sight: ${held.output}`,
  );

  writeFileSync(path.join(repo, target), "// line\n".repeat(1100));
  const shrunk = runDesktopGate({ fixtureRoot: repo, baseRef });
  assert.equal(
    shrunk.failed,
    false,
    `shrinking an inherited oversize .mjs must pass: ${shrunk.output}`,
  );

  writeFileSync(path.join(repo, target), "// line\n".repeat(1200));
  const grown = runDesktopGate({ fixtureRoot: repo, baseRef });
  assert.equal(
    grown.failed,
    true,
    "growing past the inherited size must fail the ratchet",
  );
  assert.match(grown.output, /1200 -> 1201 \(\+1\) lines \(allowed 1200\)/);
});
