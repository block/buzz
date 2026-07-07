import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChatWorkContext,
  mergeChatWorkBinding,
  shouldShowChatWorkContextContent,
} from "./chatWorkBinding.ts";

test("mergeChatWorkBinding fills an empty binding", () => {
  assert.deepEqual(
    mergeChatWorkBinding(
      null,
      {
        projectName: "Spellcheck",
        projectPath: " /Users/k/Development/sprout-spellcheck ",
        branch: " feat/spellcheck ",
        prHref: " https://github.com/block/buzz/pull/1515 ",
      },
      { now: 123 },
    ),
    {
      projectName: "Spellcheck",
      projectPath: "/Users/k/Development/sprout-spellcheck",
      branch: "feat/spellcheck",
      prHref: "https://github.com/block/buzz/pull/1515",
      prDetached: false,
      updatedAt: 123,
    },
  );
});

test("automatic hints do not replace an existing branch or PR", () => {
  const current = mergeChatWorkBinding(
    null,
    {
      branch: "feat/spellcheck",
      prHref: "https://github.com/block/buzz/pull/1515",
    },
    { now: 1 },
  );

  assert.deepEqual(
    mergeChatWorkBinding(
      current,
      {
        branch: "kennylopez-dictation",
        prHref: "https://github.com/block/buzz/pull/1511",
      },
      { now: 2 },
    ),
    {
      projectName: null,
      projectPath: null,
      branch: "feat/spellcheck",
      prHref: "https://github.com/block/buzz/pull/1515",
      prDetached: false,
      updatedAt: 2,
    },
  );
});

test("manual PR replacement can switch the bound PR", () => {
  const current = mergeChatWorkBinding(
    null,
    { prHref: "https://github.com/block/buzz/pull/1515" },
    { now: 1 },
  );

  assert.equal(
    mergeChatWorkBinding(
      current,
      { prHref: "https://github.com/block/buzz/pull/1511" },
      { now: 2, replacePr: true },
    )?.prHref,
    "https://github.com/block/buzz/pull/1511",
  );
});

test("detached PR blocks later automatic PR hints", () => {
  const detached = mergeChatWorkBinding(
    null,
    { branch: "feat/spellcheck", prHref: null, prDetached: true },
    { now: 1, replacePr: true },
  );

  assert.deepEqual(
    mergeChatWorkBinding(
      detached,
      { prHref: "https://github.com/block/buzz/pull/1511" },
      { now: 2 },
    ),
    {
      projectName: null,
      projectPath: null,
      branch: "feat/spellcheck",
      prHref: null,
      prDetached: true,
      updatedAt: 2,
    },
  );
});

test("buildChatWorkContext describes the isolated branch and PR", () => {
  const content = buildChatWorkContext({
    projectName: "Spellcheck",
    projectPath: "/Users/k/Development/sprout-spellcheck",
    branch: "feat/spellcheck",
    prHref: "https://github.com/block/buzz/pull/1515",
    prDetached: false,
    updatedAt: 1,
  });

  assert.match(content ?? "", /^Work context\nScope: this chat is isolated/m);
  assert.match(content ?? "", /Project: Spellcheck/);
  assert.match(content ?? "", /Branch: feat\/spellcheck/);
  assert.match(
    content ?? "",
    /Pull request: https:\/\/github\.com\/block\/buzz\/pull\/1515/,
  );
});

test("buildChatWorkContext skips project-only bindings", () => {
  assert.equal(
    buildChatWorkContext({
      projectName: "Spellcheck",
      projectPath: "/Users/k/Development/sprout-spellcheck",
      branch: null,
      prHref: null,
      prDetached: false,
      updatedAt: 1,
    }),
    null,
  );
});

test("shouldShowChatWorkContextContent keeps historical work context rows", () => {
  const spellcheckContext = buildChatWorkContext({
    projectName: "Spellcheck",
    projectPath: "/Users/k/Development/sprout-spellcheck",
    branch: "feat/spellcheck",
    prHref: "https://github.com/block/buzz/pull/1515",
    prDetached: false,
    updatedAt: 1,
  });
  const dictationContext = buildChatWorkContext({
    projectName: "Dictation",
    projectPath: "/Users/k/Development/sprout-dictation",
    branch: "kennylopez-dictation",
    prHref: "https://github.com/block/buzz/pull/1511",
    prDetached: false,
    updatedAt: 1,
  });

  assert.equal(
    shouldShowChatWorkContextContent(dictationContext, spellcheckContext),
    true,
  );
  assert.equal(
    shouldShowChatWorkContextContent(spellcheckContext, spellcheckContext),
    true,
  );
  assert.equal(
    shouldShowChatWorkContextContent(
      "Project setup\nFolder: /tmp/project",
      null,
    ),
    true,
  );
});
