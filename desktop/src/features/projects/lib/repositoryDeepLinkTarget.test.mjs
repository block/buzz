import assert from "node:assert/strict";
import test from "node:test";

import * as repositoryTarget from "./repositoryDeepLinkTarget.ts";
import {
  effectiveProjectRepoSource,
  projectTabForRepositoryTarget,
  repositoryTargetResultKey,
  resolveRepositoryDeepLinkTarget,
  shouldResolveRepositoryTarget,
} from "./repositoryDeepLinkTarget.ts";

const files = [
  { path: "README.md", kind: "blob" },
  { path: "GUIDES/RUNBOOK.md", kind: "blob" },
  { path: "GUIDES/setup/install.md", kind: "blob" },
];

test("resolves an exact repository file", () => {
  assert.deepEqual(
    resolveRepositoryDeepLinkTarget(files, "GUIDES/RUNBOOK.md"),
    {
      kind: "file",
      file: files[1],
      parentPath: "GUIDES",
    },
  );
});

test("resolves an existing directory from file prefixes", () => {
  assert.deepEqual(resolveRepositoryDeepLinkTarget(files, "GUIDES/setup"), {
    kind: "directory",
    path: "GUIDES/setup",
  });
});

test("does not treat a partial filename prefix as a directory", () => {
  assert.deepEqual(resolveRepositoryDeepLinkTarget(files, "GUIDES/RUN"), {
    kind: "missing",
  });
});

test("repository targets force remote snapshots while ordinary routes preserve source", () => {
  assert.equal(effectiveProjectRepoSource("local", "README.md"), "remote");
  assert.equal(effectiveProjectRepoSource("remote", "README.md"), "remote");
  assert.equal(effectiveProjectRepoSource("local", undefined), "local");
  assert.equal(effectiveProjectRepoSource("remote", undefined), "remote");
});

test("repository targets open the files tab while ordinary routes open overview", () => {
  assert.equal(projectTabForRepositoryTarget("README.md"), "files");
  assert.equal(projectTabForRepositoryTarget(undefined), "overview");
});

test("failed targets retry when the ref becomes available", () => {
  const attempt = { key: "target\0tree", outcome: "error" };
  assert.equal(
    shouldResolveRepositoryTarget({
      attempt,
      hasError: true,
      isLoading: false,
      resolutionKey: "target\0tree",
    }),
    false,
  );
  assert.equal(
    shouldResolveRepositoryTarget({
      attempt,
      hasError: false,
      isLoading: false,
      resolutionKey: "target\0tree",
    }),
    true,
  );
});

test("resolved targets retry only for a changed repository tree", () => {
  const attempt = { key: "target\0tree-a", outcome: "resolved" };
  assert.equal(
    shouldResolveRepositoryTarget({
      attempt,
      hasError: false,
      isLoading: false,
      resolutionKey: "target\0tree-a",
    }),
    false,
  );
  assert.equal(
    shouldResolveRepositoryTarget({
      attempt,
      hasError: false,
      isLoading: false,
      resolutionKey: "target\0tree-b",
    }),
    true,
  );
});

test("repository target resolution is re-armed when snapshot contents change", () => {
  const targetKey = "main\0GUIDES/RUNBOOK.md";
  assert.notEqual(
    repositoryTargetResultKey(targetKey, "commit-a\0README.md"),
    repositoryTargetResultKey(targetKey, "commit-b\0README.md"),
  );
});

test("repository target failures offer the repository root", () => {
  const onClick = () => {};
  const action = repositoryTarget.repositoryRootToastAction?.(onClick);
  assert.ok(action, "repository root action is available");
  assert.equal(action.label, "Open repository root");
  assert.equal(action.onClick, onClick);
});
