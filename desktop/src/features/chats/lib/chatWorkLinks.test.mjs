import assert from "node:assert/strict";
import test from "node:test";

import {
  collectUnambiguousPullRequestSources,
  latestUnambiguousPullRequestHref,
  latestUnambiguousPullRequestSource,
} from "./chatWorkLinks.ts";

test("latest unambiguous PR link wins", () => {
  assert.equal(
    latestUnambiguousPullRequestHref([
      { content: "Opened https://github.com/block/buzz/pull/1511" },
      { content: "Updated https://github.com/block/buzz/pull/1515" },
    ]),
    "https://github.com/block/buzz/pull/1515",
  );
});

test("latest unambiguous PR source includes the message timestamp", () => {
  const messages = [
    {
      content: "Opened https://github.com/block/buzz/pull/1511",
      created_at: 100,
    },
    {
      content: "Updated https://github.com/block/buzz/pull/1515",
      created_at: 200,
    },
  ];

  assert.deepEqual(latestUnambiguousPullRequestSource(messages), {
    href: "https://github.com/block/buzz/pull/1515",
    timestampMs: 200_000,
  });
  assert.deepEqual(collectUnambiguousPullRequestSources(messages), [
    {
      href: "https://github.com/block/buzz/pull/1511",
      timestampMs: 100_000,
    },
    {
      href: "https://github.com/block/buzz/pull/1515",
      timestampMs: 200_000,
    },
  ]);
});

test("messages mentioning several PRs are skipped as ambiguous", () => {
  assert.equal(
    latestUnambiguousPullRequestHref([
      { content: "Opened https://github.com/block/buzz/pull/1515" },
      {
        content:
          "Dictation https://github.com/block/buzz/pull/1511 and spellcheck https://github.com/block/buzz/pull/1515 are both open.",
      },
    ]),
    "https://github.com/block/buzz/pull/1515",
  );
});

test("repeated mentions of the same PR in one message count as one PR", () => {
  assert.equal(
    latestUnambiguousPullRequestHref([
      {
        content:
          "PR https://github.com/block/buzz/pull/1515 mirrors [the same PR](https://github.com/block/buzz/pull/1515).",
      },
    ]),
    "https://github.com/block/buzz/pull/1515",
  );
});

test("returns null when no unambiguous PR exists", () => {
  assert.equal(
    latestUnambiguousPullRequestHref([
      {
        content:
          "Dictation https://github.com/block/buzz/pull/1511 and spellcheck https://github.com/block/buzz/pull/1515.",
      },
    ]),
    null,
  );
});
