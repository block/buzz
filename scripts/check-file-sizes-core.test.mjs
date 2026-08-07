import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import {
  allowedLineCount,
  countLines,
  evaluateFileSize,
  parseChangedFiles,
  resolveBaseRef,
  runFileSizeCheck,
} from "./check-file-sizes-core.mjs";

function git(repo, ...args) {
  return execFileSync("git", args, { cwd: repo, encoding: "utf8" }).trim();
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

test("fileOverrides raises the ceiling for a named path without leaking to other files", async () => {
  // Build a minimal temp git repo:
  //   repo/
  //     desktop/       ← projectRoot
  //       src/
  //         governed.ts       ← 1 line over the 1000-line default ceiling
  //         exempted.ts       ← same size, but named in fileOverrides (ceiling 1200)
  //   origin/main at the initial empty commit ← base for diff
  const repoRoot = mkdtempSync(path.join(tmpdir(), "file-size-override-"));
  const projectRoot = path.join(repoRoot, "desktop");
  const srcDir = path.join(projectRoot, "src");
  mkdirSync(srcDir, { recursive: true });

  function gitRepo(...args) {
    return execFileSync("git", args, { cwd: repoRoot, encoding: "utf8" });
  }

  gitRepo("init", "-b", "main");
  gitRepo("config", "user.name", "Test");
  gitRepo("config", "user.email", "test@example.com");
  gitRepo("commit", "--allow-empty", "-m", "base");
  gitRepo("remote", "add", "origin", repoRoot);
  gitRepo("fetch", "origin", "main:refs/remotes/origin/main");

  // Add a second commit so HEAD^1 resolves (the CI path uses HEAD^1 as base,
  // and a parentless HEAD causes git cat-file -e HEAD^1^{commit} to throw).
  gitRepo("commit", "--allow-empty", "-m", "branch commit");

  // Write both files: 1001 lines each (1 over the 1000-line default ceiling)
  const content = "x\n".repeat(1001);
  writeFileSync(path.join(srcDir, "governed.ts"), content);
  writeFileSync(path.join(srcDir, "exempted.ts"), content);
  gitRepo("add", ".");

  const prevExitCode = process.exitCode;
  const errors = [];
  const origConsoleError = console.error;
  console.error = (...args) => errors.push(args.join(" "));

  try {
    await runFileSizeCheck({
      projectRoot,
      rules: [{ root: "src", extensions: new Set([".ts"]), maxLines: 1000 }],
      label: "test",
      fileOverrides: { "src/exempted.ts": 1200 },
    });

    // governed.ts (1001 lines, ceiling 1000) must violate
    assert.equal(process.exitCode, 1, "governed.ts over default ceiling should fail");
    const combined = errors.join("\n");
    assert.ok(combined.includes("governed.ts"), "violation message should name governed.ts");
    // exempted.ts (1001 lines, override ceiling 1200) must NOT be in the violation list
    assert.ok(!combined.includes("exempted.ts"), "exempted.ts under override ceiling should not appear");
  } finally {
    process.exitCode = prevExitCode;
    console.error = origConsoleError;
  }
});
