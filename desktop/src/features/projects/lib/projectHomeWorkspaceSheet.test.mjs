import assert from "node:assert/strict";
import test from "node:test";

import {
  isProjectHomeWorkspaceSheetTab,
  projectHomeWorkspaceSheetExpandTab,
  projectHomeWorkspaceSheetTitle,
} from "./projectHomeWorkspaceSheet.ts";

test("isProjectHomeWorkspaceSheetTab accepts overview workspace rows", () => {
  assert.equal(isProjectHomeWorkspaceSheetTab("issues"), true);
  assert.equal(isProjectHomeWorkspaceSheetTab("files"), true);
  assert.equal(isProjectHomeWorkspaceSheetTab("channels"), false);
  assert.equal(isProjectHomeWorkspaceSheetTab(undefined), false);
});

test("projectHomeWorkspaceSheetTitle matches overview row labels", () => {
  assert.equal(projectHomeWorkspaceSheetTitle("issues"), "Tasks");
  assert.equal(projectHomeWorkspaceSheetTitle("prs"), "Reviews");
  assert.equal(projectHomeWorkspaceSheetTitle("commits"), "Commits");
  assert.equal(projectHomeWorkspaceSheetTitle("files"), "Files");
  assert.equal(projectHomeWorkspaceSheetTitle("contributors"), "People");
});

test("projectHomeWorkspaceSheetExpandTab keeps the selected repository menu", () => {
  assert.equal(projectHomeWorkspaceSheetExpandTab("issues"), "issues");
  assert.equal(projectHomeWorkspaceSheetExpandTab("prs"), "prs");
  assert.equal(projectHomeWorkspaceSheetExpandTab("commits"), "commits");
  assert.equal(projectHomeWorkspaceSheetExpandTab("files"), "files");
  assert.equal(
    projectHomeWorkspaceSheetExpandTab("contributors"),
    "contributors",
  );
});
