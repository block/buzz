import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const SCRIPT = fileURLToPath(new URL("./stage-debug-sidecars.sh", import.meta.url));
const UNIX_SIDECARS = [
  "buzz-acp",
  "buzz-agent",
  "buzz-dev-mcp",
  "git-credential-nostr",
  "buzz",
  "buzz-backend-kubernetes",
];
const WINDOWS_SIDECARS = UNIX_SIDECARS.filter(
  (bin) => bin !== "buzz-backend-kubernetes",
);

function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), "stage-sidecars-"));
  const targetDir = path.join(root, "target");
  const destDir = path.join(root, "binaries");
  mkdirSync(path.join(targetDir, "debug"), { recursive: true });
  mkdirSync(destDir, { recursive: true });
  return { targetDir, destDir };
}

function writeSidecars(targetDir, names, suffix = "") {
  for (const bin of names) {
    writeFileSync(path.join(targetDir, "debug", `${bin}${suffix}`), `src-${bin}`);
  }
}

function seedDests(destDir, names, target, suffix = "") {
  for (const bin of names) {
    writeFileSync(
      path.join(destDir, `${bin}-${target}${suffix}`),
      `old-${bin}`,
    );
  }
}

function destContents(destDir, names, target, suffix = "") {
  return Object.fromEntries(
    names.map((bin) => [
      bin,
      readFileSync(path.join(destDir, `${bin}-${target}${suffix}`), "utf8"),
    ]),
  );
}

function run(target, targetDir, destDir) {
  return execFileSync("bash", [SCRIPT, target, targetDir, destDir], {
    encoding: "utf8",
  });
}

test("stages every unix sidecar and marks it executable", () => {
  const { targetDir, destDir } = fixture();
  const target = "aarch64-apple-darwin";
  writeSidecars(targetDir, UNIX_SIDECARS);
  run(target, targetDir, destDir);
  for (const bin of UNIX_SIDECARS) {
    const dest = path.join(destDir, `${bin}-${target}`);
    assert.equal(readFileSync(dest, "utf8"), `src-${bin}`);
    assert.equal(statSync(dest).mode & 0o111, 0o111);
  }
});

test("a missing last sidecar leaves every destination unchanged", () => {
  const { targetDir, destDir } = fixture();
  const target = "aarch64-apple-darwin";
  writeSidecars(targetDir, UNIX_SIDECARS.slice(0, -1));
  seedDests(destDir, UNIX_SIDECARS, target);
  const before = destContents(destDir, UNIX_SIDECARS, target);
  assert.throws(() => run(target, targetDir, destDir), /missing/);
  assert.deepEqual(destContents(destDir, UNIX_SIDECARS, target), before);
});

test("an empty sidecar leaves every destination unchanged", () => {
  const { targetDir, destDir } = fixture();
  const target = "aarch64-apple-darwin";
  writeSidecars(targetDir, UNIX_SIDECARS);
  writeFileSync(path.join(targetDir, "debug", "buzz-agent"), "");
  seedDests(destDir, UNIX_SIDECARS, target);
  const before = destContents(destDir, UNIX_SIDECARS, target);
  assert.throws(() => run(target, targetDir, destDir), /empty/);
  assert.deepEqual(destContents(destDir, UNIX_SIDECARS, target), before);
});

test("windows targets use .exe names and skip kubernetes", () => {
  const { targetDir, destDir } = fixture();
  const target = "x86_64-pc-windows-msvc";
  writeSidecars(targetDir, WINDOWS_SIDECARS, ".exe");
  run(target, targetDir, destDir);
  for (const bin of WINDOWS_SIDECARS) {
    assert.equal(
      readFileSync(path.join(destDir, `${bin}-${target}.exe`), "utf8"),
      `src-${bin}`,
    );
  }
  assert.throws(() =>
    readFileSync(
      path.join(destDir, `buzz-backend-kubernetes-${target}.exe`),
    ),
  );
});

test("normalizes backslashes in the cargo target directory", () => {
  const { targetDir, destDir } = fixture();
  const target = "aarch64-apple-darwin";
  writeSidecars(targetDir, UNIX_SIDECARS);
  run(target, targetDir.replaceAll("/", "\\"), destDir);
  assert.equal(
    readFileSync(path.join(destDir, `buzz-${target}`), "utf8"),
    "src-buzz",
  );
});
