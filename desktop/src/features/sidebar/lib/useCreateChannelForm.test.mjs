import assert from "node:assert/strict";
import test from "node:test";

import { resolveInitialChannelTemplate } from "./useCreateChannelForm.ts";

const template = {
  id: "review-template",
  name: "Review",
  description: "Review changes",
  channelType: "stream",
  visibility: "private",
  canvasTemplate: null,
  projectFolders: [],
  projectFolder: null,
  worktree: null,
  agents: { personas: [], teams: [] },
  isBuiltin: false,
  createdAt: "now",
  updatedAt: "now",
};

test("keeps a section template locked while the template query is pending", () => {
  assert.equal(
    resolveInitialChannelTemplate(template.id, [], "pending"),
    undefined,
  );
});

test("applies a section template after a successful query", () => {
  assert.equal(
    resolveInitialChannelTemplate(template.id, [template], "success"),
    template,
  );
});

test("clears a missing section template after a successful query", () => {
  assert.equal(resolveInitialChannelTemplate(template.id, [], "success"), null);
});

test("clears a locked section template after a terminal query failure", () => {
  assert.equal(resolveInitialChannelTemplate(template.id, [], "error"), null);
});
