import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const REPO_A = {
  branchOptions: ["main", "release"],
  defaultBranch: "main",
  projectAvailable: true,
  projectPending: false,
  repositoryId: "30617:owner:repo-a",
  tags: [{ name: "v1" }],
};

const REPO_B = {
  branchOptions: ["trunk", "release"],
  defaultBranch: "trunk",
  projectAvailable: true,
  projectPending: false,
  repositoryId: "30617:owner:repo-b",
  tags: [{ name: "v1" }],
};

async function renderSelection(initialProps) {
  const { act, renderHook } = await import("@testing-library/react");
  const rendered = renderHook(
    (props) => {
      const { useProjectRepositoryRefSelection } = hookModule;
      return useProjectRepositoryRefSelection(props);
    },
    { initialProps },
  );
  return { act, ...rendered };
}

let hookModule;
before(async () => {
  hookModule = await import("./useProjectRepositoryRefSelection.ts");
});

test("a repository switch resets a same-named branch selection to the new default", async () => {
  const { act, rerender, result } = await renderSelection(REPO_A);

  act(() => result.current.selectBranch("release"));
  assert.equal(result.current.activeBranch, "release");

  // Repo B also has a `release` branch — it must NOT survive the switch.
  rerender(REPO_B);
  assert.equal(result.current.activeBranch, "trunk");
  assert.equal(result.current.selectedTag, null);
});

test("a repository switch resets a same-named tag selection", async () => {
  const { act, rerender, result } = await renderSelection(REPO_A);

  act(() => result.current.selectTag("v1"));
  assert.equal(result.current.selectedTag, "v1");

  // Repo B also has a `v1` tag — it must NOT survive the switch.
  rerender(REPO_B);
  assert.equal(result.current.selectedTag, null);
  assert.equal(result.current.activeBranch, "trunk");
});

test("selections survive option refreshes within the same repository", async () => {
  const { act, rerender, result } = await renderSelection(REPO_A);

  act(() => result.current.selectBranch("release"));
  act(() => result.current.selectTag("v1"));

  // Same repository, new option arrays (a refetch) — selection is kept.
  rerender({ ...REPO_A, branchOptions: ["main", "release", "feature"] });
  assert.equal(result.current.activeBranch, "release");
  assert.equal(result.current.selectedTag, "v1");
});
