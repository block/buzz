import assert from "node:assert/strict";
import { test } from "node:test";

import {
  activateTab,
  activeTab,
  closeTab,
  closeTabsUnder,
  dirtyPaths,
  emptyTabsState,
  hasDirtyTabs,
  markTabSaved,
  openTab,
  reloadTabFromDisk,
  renameTabPath,
  setTabContent,
  setTabViewMode,
  tabLabel,
} from "./documentTabs.ts";

function makeTab(path, overrides = {}) {
  return {
    content: "body",
    diskContent: "body",
    frontmatter: null,
    isDirty: false,
    name: tabLabel(path),
    path,
    roundTrip: "stable",
    viewMode: "live",
    ...overrides,
  };
}

function withTabs(...paths) {
  return paths.reduce(
    (state, path) => openTab(state, makeTab(path)),
    emptyTabsState,
  );
}

test("tabLabel strips the directory and the markdown extension", () => {
  assert.equal(tabLabel("/vault/Notes/Meeting notes.md"), "Meeting notes");
  assert.equal(tabLabel("/vault/legacy.markdown"), "legacy");
});

test("opening tabs appends and focuses the new one", () => {
  const state = withTabs("/v/a.md", "/v/b.md");
  assert.deepEqual(
    state.tabs.map((t) => t.path),
    ["/v/a.md", "/v/b.md"],
  );
  assert.equal(state.activePath, "/v/b.md");
});

test("re-opening an open file focuses it without clobbering its buffer", () => {
  let state = withTabs("/v/a.md", "/v/b.md");
  state = setTabContent(state, "/v/a.md", "edited", "edited");
  state = openTab(state, makeTab("/v/a.md"));

  assert.equal(state.activePath, "/v/a.md");
  assert.equal(state.tabs.length, 2, "must not duplicate the tab");
  const reopened = state.tabs.find((t) => t.path === "/v/a.md");
  assert.equal(reopened.content, "edited", "unsaved edits must survive");
  assert.equal(reopened.isDirty, true);
});

test("closing the active tab focuses its right-hand neighbour", () => {
  let state = withTabs("/v/a.md", "/v/b.md", "/v/c.md");
  state = activateTab(state, "/v/b.md");
  state = closeTab(state, "/v/b.md");
  assert.equal(state.activePath, "/v/c.md");
});

test("closing the last tab falls back to the left", () => {
  let state = withTabs("/v/a.md", "/v/b.md", "/v/c.md");
  // /v/c.md is already active as the most recently opened.
  state = closeTab(state, "/v/c.md");
  assert.equal(state.activePath, "/v/b.md");
});

test("closing an inactive tab leaves focus alone", () => {
  let state = withTabs("/v/a.md", "/v/b.md", "/v/c.md");
  state = closeTab(state, "/v/a.md");
  assert.equal(state.activePath, "/v/c.md");
  assert.equal(state.tabs.length, 2);
});

test("closing the only tab empties the state", () => {
  const state = closeTab(withTabs("/v/a.md"), "/v/a.md");
  assert.deepEqual(state, emptyTabsState);
  assert.equal(activeTab(state), null);
});

test("closing an unknown path is a no-op", () => {
  const state = withTabs("/v/a.md");
  assert.equal(closeTab(state, "/v/missing.md"), state);
});

test("dirtiness is derived from the disk projection, not latched", () => {
  let state = withTabs("/v/a.md");
  state = setTabContent(state, "/v/a.md", "edited", "edited");
  assert.equal(activeTab(state).isDirty, true);
  assert.deepEqual(dirtyPaths(state), ["/v/a.md"]);

  // Undoing back to the on-disk text clears the flag rather than leaving the
  // tab permanently dirty.
  state = setTabContent(state, "/v/a.md", "body", "body");
  assert.equal(activeTab(state).isDirty, false);
  assert.equal(hasDirtyTabs(state), false);
});

test("the disk projection drives dirtiness, not the editor buffer", () => {
  // Frontmatter lives outside `content`, so the projection can differ from it.
  let state = openTab(
    emptyTabsState,
    makeTab("/v/a.md", {
      content: "body",
      diskContent: "---\na: 1\n---\n\nbody",
      frontmatter: "---\na: 1\n---\n\n",
    }),
  );
  // Same body → same bytes on disk → clean.
  state = setTabContent(state, "/v/a.md", "body", "---\na: 1\n---\n\nbody");
  assert.equal(activeTab(state).isDirty, false);
});

test("markTabSaved adopts the written bytes and clears dirtiness", () => {
  let state = withTabs("/v/a.md");
  state = setTabContent(state, "/v/a.md", "edited", "edited");
  state = markTabSaved(state, "/v/a.md", "edited");
  const tab = activeTab(state);
  assert.equal(tab.isDirty, false);
  assert.equal(tab.diskContent, "edited");
});

test("reloadTabFromDisk discards local edits", () => {
  let state = withTabs("/v/a.md");
  state = setTabContent(state, "/v/a.md", "mine", "mine");
  state = reloadTabFromDisk(state, "/v/a.md", {
    content: "theirs",
    diskContent: "theirs",
    frontmatter: null,
    roundTrip: "stable",
  });
  const tab = activeTab(state);
  assert.equal(tab.content, "theirs");
  assert.equal(tab.isDirty, false);
});

test("view mode is per tab", () => {
  let state = withTabs("/v/a.md", "/v/b.md");
  state = setTabViewMode(state, "/v/a.md", "source");
  assert.equal(state.tabs.find((t) => t.path === "/v/a.md").viewMode, "source");
  assert.equal(state.tabs.find((t) => t.path === "/v/b.md").viewMode, "live");
});

test("renaming a file follows its open tab and keeps focus", () => {
  let state = withTabs("/v/a.md", "/v/b.md");
  state = activateTab(state, "/v/a.md");
  state = renameTabPath(state, "/v/a.md", "/v/renamed.md");

  assert.equal(state.activePath, "/v/renamed.md");
  const tab = state.tabs.find((t) => t.path === "/v/renamed.md");
  assert.equal(tab.name, "renamed");
  assert.equal(
    state.tabs.some((t) => t.path === "/v/a.md"),
    false,
  );
});

test("renaming an unopened file is a no-op", () => {
  const state = withTabs("/v/a.md");
  assert.equal(renameTabPath(state, "/v/other.md", "/v/x.md"), state);
});

test("deleting a folder closes every tab beneath it", () => {
  let state = withTabs("/v/Notes/a.md", "/v/Notes/deep/b.md", "/v/top.md");
  state = closeTabsUnder(state, "/v/Notes");

  assert.deepEqual(
    state.tabs.map((t) => t.path),
    ["/v/top.md"],
  );
  assert.equal(state.activePath, "/v/top.md");
});

test("closeTabsUnder does not match a sibling sharing a name prefix", () => {
  let state = withTabs("/v/Notes/a.md", "/v/Notes-archive/b.md");
  state = closeTabsUnder(state, "/v/Notes");
  assert.deepEqual(
    state.tabs.map((t) => t.path),
    ["/v/Notes-archive/b.md"],
  );
});
