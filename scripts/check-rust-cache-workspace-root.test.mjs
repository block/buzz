import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import { dependenciesUnderWorkspaceRoot } from "./check-rust-cache-workspace-root.mjs";

const script = fileURLToPath(
  new URL("./check-rust-cache-workspace-root.mjs", import.meta.url),
);

function metadata(registry) {
  return {
    workspace_root: "/work/buzz",
    workspace_members: ["path+file:///work/buzz/crates/buzz-core#0.1.0"],
    packages: [
      {
        id: "path+file:///work/buzz/crates/buzz-core#0.1.0",
        name: "buzz-core",
        version: "0.1.0",
        manifest_path: "/work/buzz/crates/buzz-core/Cargo.toml",
      },
      {
        id: "registry+https://github.com/rust-lang/crates.io-index#serde@1.0.0",
        name: "serde",
        version: "1.0.0",
        manifest_path: `${registry}/registry/src/index.crates.io-1949cf8c6b5b557f/serde-1.0.0/Cargo.toml`,
      },
    ],
  };
}

function check(meta) {
  return spawnSync("node", [script], {
    input: JSON.stringify(meta),
    encoding: "utf8",
  });
}

test("a registry outside the checkout passes", () => {
  const meta = metadata("/home/runner/.cargo-hermit");
  assert.deepEqual(dependenciesUnderWorkspaceRoot(meta), []);
  const result = check(meta);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /1 dependency package\(s\) outside/);
});

test("Hermit's default in-checkout registry fails", () => {
  const meta = metadata("/work/buzz/.hermit/rust");
  assert.deepEqual(dependenciesUnderWorkspaceRoot(meta), [
    "serde 1.0.0 (/work/buzz/.hermit/rust/registry/src/index.crates.io-1949cf8c6b5b557f/serde-1.0.0/Cargo.toml)",
  ]);
  const result = check(meta);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /1 non-member package\(s\) have a manifest under \/work\/buzz/);
});

test("a sibling directory sharing the root prefix fails, as rust-cache sees it", () => {
  // rust-cache's startsWith has no separator, so /work/buzz-cargo is "inside".
  assert.deepEqual(dependenciesUnderWorkspaceRoot(metadata("/work/buzz-cargo")), [
    "serde 1.0.0 (/work/buzz-cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde-1.0.0/Cargo.toml)",
  ]);
});
