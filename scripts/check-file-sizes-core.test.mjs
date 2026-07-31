import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import {
  allowedLineCount,
  allowedLineCountFromBases,
  countLines,
  evaluateFileSize,
  parseChangedFiles,
  resolveBaseRef,
  resolveBaseRefs,
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

test("an in-progress merge treats both parents as accepted file-size baselines", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "file-size-merge-base-"));
  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  git(repo, "commit", "--allow-empty", "-m", "base");
  git(repo, "switch", "-c", "upstream");
  git(repo, "commit", "--allow-empty", "-m", "upstream growth");
  const upstream = git(repo, "rev-parse", "HEAD");
  git(repo, "switch", "main");
  git(repo, "commit", "--allow-empty", "-m", "downstream work");
  const downstream = git(repo, "rev-parse", "HEAD");
  git(repo, "merge", "--no-ff", "--no-commit", "upstream");

  assert.deepEqual(resolveBaseRefs(repo, {}), [downstream, upstream]);
});

test("a committed merge keeps both parents as accepted file-size baselines", () => {
  const repo = mkdtempSync(path.join(tmpdir(), "file-size-merge-commit-"));
  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  git(repo, "commit", "--allow-empty", "-m", "base");
  git(repo, "switch", "-c", "upstream");
  git(repo, "commit", "--allow-empty", "-m", "upstream growth");
  const upstream = git(repo, "rev-parse", "HEAD");
  git(repo, "switch", "main");
  git(repo, "commit", "--allow-empty", "-m", "downstream work");
  const downstream = git(repo, "rev-parse", "HEAD");
  git(repo, "merge", "--no-ff", "upstream", "-m", "merge upstream");

  assert.deepEqual(resolveBaseRefs(repo, {}), [downstream, upstream]);
});

test("a shallow-style merge without a common ancestor still checks parent limits", async () => {
  const repo = mkdtempSync(path.join(tmpdir(), "file-size-shallow-merge-"));
  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  mkdirSync(path.join(repo, "web", "src"), { recursive: true });
  writeFileSync(path.join(repo, "web", "src", "small.ts"), "one\ntwo\n");
  git(repo, "add", ".");
  git(repo, "commit", "-m", "first parent");
  const firstParent = git(repo, "rev-parse", "HEAD");
  const tree = git(repo, "rev-parse", "HEAD^{tree}");
  const secondParent = git(repo, "commit-tree", tree, "-m", "second root");
  const merge = git(
    repo,
    "commit-tree",
    tree,
    "-p",
    firstParent,
    "-p",
    secondParent,
    "-m",
    "merge without available ancestry",
  );
  git(repo, "reset", "--hard", merge);

  await assert.doesNotReject(() =>
    runFileSizeCheck({
      projectRoot: path.join(repo, "web"),
      rules: [{ root: "src", extensions: new Set([".ts"]), maxLines: 10 }],
      label: "web",
    }),
  );
});

test("a merge accepts an oversized file already present in either parent", () => {
  assert.equal(allowedLineCountFromBases([900, 1232], 1000), 1232);
  assert.equal(allowedLineCountFromBases([null, 1232], 1000), 1232);
  assert.equal(allowedLineCountFromBases([900, null], 1000), 1000);
});

test("a downstream override remains an explicit upper bound after a merge", () => {
  assert.equal(allowedLineCountFromBases([900, 1232], 1000, 1260), 1260);
  assert.equal(allowedLineCountFromBases([900, 1232], 1000, 1200), 1232);
});

test("independent parent growth may combine without becoming merge-time debt", () => {
  assert.equal(
    allowedLineCountFromBases([938, 995], 1000, undefined, 915),
    1018,
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
