import assert from "node:assert/strict";
import test from "node:test";

import {
  buildProjectSetupContext,
  deriveBranchTitle,
  deriveChatTitle,
  deriveConversationTitle,
  uniqueMentionPubkeys,
} from "./chatSetup.ts";

test("deriveChatTitle trims and shortens long first prompts", () => {
  assert.equal(
    deriveChatTitle("  build me a dashboard  "),
    "build me a dashboard",
  );
  assert.equal(deriveChatTitle("x".repeat(90)).length, 72);
});

test("deriveConversationTitle strips conversational lead-ins", () => {
  assert.equal(
    deriveConversationTitle(
      "Hey Fizz, can you help me figure out why my Playwright tests are flaky?",
    ),
    "Figure out why my Playwright tests are flaky",
  );
  assert.equal(
    deriveConversationTitle(
      "please add dark mode support to the settings page",
    ),
    "Add dark mode support to the settings page",
  );
  assert.equal(
    deriveConversationTitle("I want to rename the deploy workflow"),
    "Rename the deploy workflow",
  );
});

test("deriveConversationTitle keeps only the opening sentence", () => {
  assert.equal(
    deriveConversationTitle(
      "Why is the relay slow today? Also can you check the logs and maybe restart it.",
    ),
    "Why is the relay slow today",
  );
});

test("deriveConversationTitle caps on a word boundary", () => {
  const title = deriveConversationTitle(
    "investigate the intermittent websocket disconnects happening on the staging relay every afternoon",
  );
  assert.ok(title.length <= 48, `too long: ${title}`);
  assert.ok(!title.endsWith(" "), "trailing space");
  assert.equal(title, "Investigate the intermittent websocket");
});

test("deriveConversationTitle strips markdown and URLs", () => {
  assert.equal(
    deriveConversationTitle(
      "look at **this** `render bug` https://example.com/issue/42",
    ),
    "Look at this render bug",
  );
});

test("deriveConversationTitle falls back when nothing survives", () => {
  assert.equal(deriveConversationTitle("hey!"), "hey!");
  assert.equal(deriveConversationTitle("  "), "New chat");
});

test("deriveBranchTitle turns owner-prefixed branches into readable titles", () => {
  assert.equal(deriveBranchTitle("kennylopez-welcome-icon"), "Welcome icon");
  assert.equal(deriveBranchTitle("origin/kennylopez-chat-mode"), "Chat mode");
  assert.equal(
    deriveBranchTitle("refs/heads/kennethlopez-fix-activate-agent-banner"),
    "Activate agent banner",
  );
});

test("deriveBranchTitle strips common work prefixes and ticket keys", () => {
  assert.equal(deriveBranchTitle("feat/spellcheck"), "Spellcheck");
  assert.equal(deriveBranchTitle("fix/panel-width"), "Panel width");
  assert.equal(deriveBranchTitle("ABC-123-copy-link-mode"), "Copy link mode");
  assert.equal(deriveBranchTitle("wip"), null);
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
