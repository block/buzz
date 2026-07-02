import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectSetupContext,
  deriveChatTitle,
  uniqueMentionPubkeys,
} from "./chatSetup.ts";

test("deriveChatTitle trims and shortens long first prompts", () => {
  assert.equal(
    deriveChatTitle("  build me a dashboard  "),
    "build me a dashboard",
  );
  assert.equal(deriveChatTitle("x".repeat(90)).length, 72);
});

test("uniqueMentionPubkeys adds default agent and removes sender", () => {
  const self =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  const agent =
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

  assert.deepEqual(uniqueMentionPubkeys(self, [self], agent), [agent]);
});

test("buildProjectSetupContext captures scratch project setup", () => {
  const context = buildProjectSetupContext({
    project: {
      id: "project-1",
      name: "SizzleStudio",
      path: "/Users/me/Development/sizzle",
      templateId: "template-1",
      updatedAt: 1,
      chatCount: 1,
    },
    templateName: "Launch plan",
    agent: {
      name: "Fizz",
    },
  });

  assert.equal(
    context,
    [
      "Project setup",
      "Project: SizzleStudio",
      "Folder: /Users/me/Development/sizzle",
      "Template: Launch plan",
      "Agent: Fizz",
    ].join("\n"),
  );
});

test("buildProjectSetupContext captures no-project template setup", () => {
  assert.equal(
    buildProjectSetupContext({
      templateName: "Launch plan",
    }),
    ["Project setup", "Project: none", "Template: Launch plan"].join("\n"),
  );
});

test("buildProjectSetupContext omits empty setup context", () => {
  assert.equal(buildProjectSetupContext({}), null);
});
