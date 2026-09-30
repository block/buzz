import assert from "node:assert/strict";
import test from "node:test";

import {
  parseProjectDetailSearch,
  wantsProjectRepositorySurface,
} from "./projectDetailSearch.ts";

test("parseProjectDetailSearch keeps forge params and channel panel params", () => {
  const search = parseProjectDetailSearch({
    repositoryId: "30617:owner:buzz",
    tab: "files",
    filePath: "src/main.ts",
    thread: "abc123",
    agentSession: "def456",
    channelManagement: "1",
    extra: "dropped",
  });

  assert.equal(search.repositoryId, "30617:owner:buzz");
  assert.equal(search.tab, "files");
  assert.equal(search.filePath, "src/main.ts");
  assert.equal(search.thread, "abc123");
  assert.equal(search.agentSession, "def456");
  assert.equal(search.channelManagement, "1");
  assert.equal("extra" in search, false);
});

test("parseProjectDetailSearch drops empty channel panel params", () => {
  const search = parseProjectDetailSearch({
    thread: "",
    messageId: "",
    tab: "not-a-tab",
  });

  assert.equal(search.thread, undefined);
  assert.equal(search.messageId, undefined);
  assert.equal(search.tab, undefined);
});

test("wantsProjectRepositorySurface is false for channel-first project home", () => {
  assert.equal(
    wantsProjectRepositorySurface({
      projectId: "30621:owner:platform",
    }),
    false,
  );
});

test("wantsProjectRepositorySurface is true for repo, tab, or work-item params", () => {
  assert.equal(
    wantsProjectRepositorySurface({
      projectId: "30621:owner:platform",
      repositoryId: "30617:owner:buzz",
    }),
    true,
  );
  assert.equal(
    wantsProjectRepositorySurface({
      projectId: "30621:owner:platform",
      tab: "files",
    }),
    true,
  );
  assert.equal(
    wantsProjectRepositorySurface({
      filePath: "src/main.ts",
      projectId: "30621:owner:platform",
    }),
    true,
  );
});

test("wantsProjectRepositorySurface is true for a legacy kind:30617 project id", () => {
  assert.equal(
    wantsProjectRepositorySurface({
      projectId: "30617:owner:buzz",
    }),
    true,
  );
});
