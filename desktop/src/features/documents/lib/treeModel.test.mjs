import assert from "node:assert/strict";
import { test } from "node:test";

import {
  ancestorFolderPaths,
  baseName,
  canMoveInto,
  collectFilePaths,
  findEntry,
  flattenVisibleRows,
  joinPath,
  parentOf,
  relativeTo,
  stripMarkdownExtension,
} from "./treeModel.ts";

/** <vault>/Notes/{plain.md,Deep/buried.md} plus <vault>/top.md */
function fixture() {
  return [
    {
      name: "Notes",
      path: "/vault/Notes",
      isDirectory: true,
      children: [
        {
          name: "Deep",
          path: "/vault/Notes/Deep",
          isDirectory: true,
          children: [
            {
              name: "buried.md",
              path: "/vault/Notes/Deep/buried.md",
              isDirectory: false,
              children: null,
            },
          ],
        },
        {
          name: "plain.md",
          path: "/vault/Notes/plain.md",
          isDirectory: false,
          children: null,
        },
      ],
    },
    {
      name: "top.md",
      path: "/vault/top.md",
      isDirectory: false,
      children: null,
    },
  ];
}

test("strips both markdown extensions, case-insensitively", () => {
  assert.equal(stripMarkdownExtension("plain.md"), "plain");
  assert.equal(stripMarkdownExtension("legacy.MARKDOWN"), "legacy");
  assert.equal(stripMarkdownExtension("no-extension"), "no-extension");
  assert.equal(stripMarkdownExtension("dotted.name.md"), "dotted.name");
});

test("baseName and parentOf handle both separators and trailing slashes", () => {
  assert.equal(baseName("/vault/Notes/plain.md"), "plain.md");
  assert.equal(baseName("/vault/Notes/"), "Notes");
  assert.equal(baseName("C:\\vault\\Notes\\plain.md"), "plain.md");
  assert.equal(parentOf("/vault/Notes/plain.md"), "/vault/Notes");
  assert.equal(parentOf("C:\\vault\\Notes\\plain.md"), "C:\\vault\\Notes");
  assert.equal(parentOf("/top.md"), "/");
});

test("joinPath preserves the parent's separator", () => {
  assert.equal(joinPath("/vault/Notes", "new.md"), "/vault/Notes/new.md");
  assert.equal(joinPath("/vault/Notes/", "new.md"), "/vault/Notes/new.md");
  assert.equal(joinPath("C:\\vault", "new.md"), "C:\\vault\\new.md");
});

test("relativeTo strips the root, and passes through outside paths", () => {
  assert.equal(relativeTo("/vault", "/vault/Notes/plain.md"), "Notes/plain.md");
  assert.equal(relativeTo("/vault/", "/vault/top.md"), "top.md");
  assert.equal(relativeTo("/vault", "/elsewhere/x.md"), "/elsewhere/x.md");
});

test("flattenVisibleRows omits the subtrees of collapsed folders", () => {
  const tree = fixture();

  const collapsed = flattenVisibleRows(tree, new Set());
  assert.deepEqual(
    collapsed.map((row) => row.entry.path),
    ["/vault/Notes", "/vault/top.md"],
  );

  const oneOpen = flattenVisibleRows(tree, new Set(["/vault/Notes"]));
  assert.deepEqual(
    oneOpen.map((row) => row.entry.path),
    [
      "/vault/Notes",
      "/vault/Notes/Deep",
      "/vault/Notes/plain.md",
      "/vault/top.md",
    ],
  );

  const bothOpen = flattenVisibleRows(
    tree,
    new Set(["/vault/Notes", "/vault/Notes/Deep"]),
  );
  assert.deepEqual(
    bothOpen.map((row) => row.entry.path),
    [
      "/vault/Notes",
      "/vault/Notes/Deep",
      "/vault/Notes/Deep/buried.md",
      "/vault/Notes/plain.md",
      "/vault/top.md",
    ],
  );
  assert.deepEqual(
    bothOpen.map((row) => row.depth),
    [0, 1, 2, 1, 0],
  );
});

test("ancestorFolderPaths lists the folders to expand to reveal a file", () => {
  assert.deepEqual(
    ancestorFolderPaths("/vault", "/vault/Notes/Deep/buried.md"),
    ["/vault/Notes", "/vault/Notes/Deep"],
  );
  assert.deepEqual(ancestorFolderPaths("/vault", "/vault/top.md"), []);
});

test("collectFilePaths walks depth-first and skips directories", () => {
  assert.deepEqual(collectFilePaths(fixture()), [
    "/vault/Notes/Deep/buried.md",
    "/vault/Notes/plain.md",
    "/vault/top.md",
  ]);
});

test("findEntry locates nested entries and reports misses", () => {
  const tree = fixture();
  assert.equal(
    findEntry(tree, "/vault/Notes/Deep/buried.md")?.name,
    "buried.md",
  );
  assert.equal(findEntry(tree, "/vault/Notes")?.isDirectory, true);
  assert.equal(findEntry(tree, "/vault/missing.md"), null);
});

test("canMoveInto rejects no-ops and moves into a folder's own subtree", () => {
  // Moving a folder into itself or a descendant would orphan it.
  assert.equal(canMoveInto("/vault/Notes", "/vault/Notes"), false);
  assert.equal(canMoveInto("/vault/Notes", "/vault/Notes/Deep"), false);
  // Already in the destination — nothing to do.
  assert.equal(canMoveInto("/vault/Notes/plain.md", "/vault/Notes"), false);
  // Legitimate moves.
  assert.equal(canMoveInto("/vault/Notes/plain.md", "/vault"), true);
  assert.equal(canMoveInto("/vault/Notes/Deep", "/vault"), true);
  // A sibling that merely shares a name prefix is not a descendant.
  assert.equal(canMoveInto("/vault/Notes", "/vault/Notes-archive"), true);
});
