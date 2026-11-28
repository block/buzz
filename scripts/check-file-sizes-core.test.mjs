import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { after } from "node:test";
import { policy as mobilePolicy } from "../mobile/scripts/check-file-sizes.mjs";
import { policy as webPolicy } from "../web/scripts/check-file-sizes.mjs";
import {
  allowedLineCount,
  countLines,
  evaluateFileSize,
  parseChangedFiles,
  resolveBaseRef,
} from "./check-file-sizes-core.mjs";

const repoRoot = path.resolve(import.meta.dirname, "..");

const fixtureRepos = [];
after(() => {
  for (const repo of fixtureRepos)
    rmSync(repo, { recursive: true, force: true });
});

function createFixtureRepo(prefix) {
  const repo = realpathSync(mkdtempSync(path.join(tmpdir(), prefix)));
  fixtureRepos.push(repo);
  return repo;
}

function git(repo, ...args) {
  // These fixture repositories inherit both hook configuration and Git's
  // repository-local environment when this test runs from pre-push. Isolate
  // them completely so fixture commits cannot recurse into the real checkout.
  // A global core.fsmonitor would also leave a daemon running per fixture.
  const env = Object.fromEntries(
    Object.entries(process.env).filter(([key]) => !key.startsWith("GIT_")),
  );
  return execFileSync(
    "git",
    ["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false", ...args],
    {
      cwd: repo,
      encoding: "utf8",
      env,
    },
  ).trim();
}

function createEntrypointFixture({
  surface,
  files,
  lineDelta = 1,
  symlinkEntrypoint = false,
}) {
  const repo = createFixtureRepo(`file-size-${surface}-`);
  const scriptsDir = path.join(repo, "scripts");
  const surfaceScriptsDir = path.join(repo, surface, "scripts");
  mkdirSync(scriptsDir, { recursive: true });
  mkdirSync(surfaceScriptsDir, { recursive: true });
  copyFileSync(
    path.join(repoRoot, "scripts/check-file-sizes-core.mjs"),
    path.join(scriptsDir, "check-file-sizes-core.mjs"),
  );
  for (const fileName of ["check-file-sizes.mjs", "file-size-policy.mjs"]) {
    copyFileSync(
      path.join(repoRoot, surface, "scripts", fileName),
      path.join(surfaceScriptsDir, fileName),
    );
  }

  git(repo, "init", "-b", "main");
  git(repo, "config", "user.name", "Test");
  git(repo, "config", "user.email", "test@example.com");
  git(repo, "add", ".");
  git(repo, "commit", "-m", "base");
  const base = git(repo, "rev-parse", "HEAD");
  git(repo, "switch", "-c", "feature");

  for (const { relativeFile, maxLines } of files) {
    const governedFile = path.join(repo, surface, relativeFile);
    const lineCount = maxLines + lineDelta;
    mkdirSync(path.dirname(governedFile), { recursive: true });
    writeFileSync(governedFile, `${"line\n".repeat(lineCount - 1)}line`);
  }

  const realEntrypointPath = path.join(
    surfaceScriptsDir,
    "check-file-sizes.mjs",
  );
  let entrypointPath = realEntrypointPath;
  if (symlinkEntrypoint) {
    entrypointPath = path.join(repo, `${surface}-file-size-check.mjs`);
    symlinkSync(realEntrypointPath, entrypointPath);
  }
  const result = spawnSync(realpathSync(process.execPath), [entrypointPath], {
    cwd: repo,
    encoding: "utf8",
    env: { ...process.env, CHECK_FILE_SIZES_BASE: base },
  });
  return { result, relativeFiles: files.map(({ relativeFile }) => relativeFile) };
}

test("local base resolution uses the branch merge-base and fails without origin/main", () => {
  const repo = createFixtureRepo("file-size-base-");
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

const entrypointCases = [
  {
    surface: "mobile",
    files: [{ relativeFile: "lib/oversized.dart", maxLines: 1200 }],
  },
  {
    surface: "web",
    files: [
      { relativeFile: "src/app/oversized.ts", maxLines: 1000 },
      { relativeFile: "src/features/oversized.tsx", maxLines: 1000 },
      { relativeFile: "src/shared/api/oversized.ts", maxLines: 1000 },
    ],
  },
];

test("surface entrypoints execute every production rule", () => {
  for (const fixture of entrypointCases) {
    const { result, relativeFiles } = createEntrypointFixture(fixture);
    assert.equal(
      result.status,
      1,
      `${fixture.surface} should reject every ceiling + 1: ${result.stderr || result.stdout}`,
    );
    for (const relativeFile of relativeFiles) {
      assert.ok(
        result.stderr.includes(relativeFile),
        `${fixture.surface} should report ${relativeFile}: ${result.stderr}`,
      );
    }
  }
});

test("surface entrypoints execute through symlinked paths", () => {
  for (const fixture of entrypointCases) {
    const { result, relativeFiles } = createEntrypointFixture({
      ...fixture,
      symlinkEntrypoint: true,
    });
    assert.equal(
      result.status,
      1,
      `${fixture.surface} symlink should reject ceiling + 1: ${result.stderr || result.stdout}`,
    );
    for (const relativeFile of relativeFiles) {
      assert.ok(
        result.stderr.includes(relativeFile),
        `${fixture.surface} symlink should report ${relativeFile}: ${result.stderr}`,
      );
    }
  }
});

test("surface entrypoints allow every production rule at its ceiling", () => {
  for (const fixture of entrypointCases) {
    const { result } = createEntrypointFixture({ ...fixture, lineDelta: 0 });
    assert.equal(
      result.status,
      0,
      `${fixture.surface} should allow every ceiling: ${result.stderr || result.stdout}`,
    );
  }
});

test("counts empty, LF, and CRLF content with the existing semantics", () => {
  assert.equal(countLines(""), 0);
  assert.equal(countLines("one\n"), 2);
  assert.equal(countLines("one\r\ntwo"), 2);
});

test("surface entrypoints expose the exact ordered production policies", () => {
  const policies = [
    [mobilePolicy, [["lib", [".dart"], 1200]]],
    [
      webPolicy,
      [
        ["src/app", [".ts", ".tsx"], 1000],
        ["src/features", [".ts", ".tsx"], 1000],
        ["src/shared/api", [".ts", ".tsx"], 1000],
      ],
    ],
  ];

  for (const [policy, expectedRules] of policies) {
    const actualRules = policy.rules.map((rule) => [
      rule.root,
      [...rule.extensions],
      rule.maxLines,
    ]);
    assert.deepEqual(
      actualRules,
      expectedRules,
      `${policy.label} production rules`,
    );

    for (const rule of policy.rules) {
      assert.equal(
        evaluateFileSize({
          baseLines: null,
          candidateLines: rule.maxLines,
          maxLines: rule.maxLines,
        }).violates,
        false,
        `${policy.label} ${rule.root} should allow the ceiling`,
      );
      assert.equal(
        evaluateFileSize({
          baseLines: null,
          candidateLines: rule.maxLines + 1,
          maxLines: rule.maxLines,
        }).violates,
        true,
        `${policy.label} ${rule.root} should reject ceiling + 1`,
      );
    }
  }
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
      "M\0web/src/a.ts\0D\0web/src/b.ts\0R100\0web/src/old.ts\0web/src/new.ts\0",
    ),
    [
      { status: "M", path: "web/src/a.ts" },
      { status: "D", path: "web/src/b.ts" },
      {
        status: "R",
        oldPath: "web/src/old.ts",
        path: "web/src/new.ts",
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
