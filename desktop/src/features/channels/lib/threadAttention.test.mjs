import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  areThreadAttentionRowsEqual,
  buildHeadPreview,
  buildThreadAttentionRows,
  formatCoarseUptime,
  totalUnreadCount,
} from "./threadAttention.ts";

describe("formatCoarseUptime", () => {
  it("renders whole seconds under a minute", () => {
    assert.equal(formatCoarseUptime(0), "0s");
    assert.equal(formatCoarseUptime(999), "0s");
    assert.equal(formatCoarseUptime(45_000), "45s");
    assert.equal(formatCoarseUptime(59_999), "59s");
  });

  it("renders whole minutes under an hour, no seconds", () => {
    assert.equal(formatCoarseUptime(60_000), "1m");
    assert.equal(formatCoarseUptime(3 * 60_000 + 12_000), "3m");
    assert.equal(formatCoarseUptime(59 * 60_000 + 59_000), "59m");
  });

  it("renders whole hours beyond an hour, no minutes", () => {
    assert.equal(formatCoarseUptime(60 * 60_000), "1h");
    assert.equal(formatCoarseUptime(2 * 60 * 60_000 + 31 * 60_000), "2h");
  });

  it("clamps negative durations to 0s", () => {
    assert.equal(formatCoarseUptime(-5_000), "0s");
  });
});

describe("buildHeadPreview", () => {
  it("collapses whitespace to one line", () => {
    assert.equal(buildHeadPreview("a\nb\t c"), "a b c");
  });

  it("returns null for blank bodies", () => {
    assert.equal(buildHeadPreview("   \n "), null);
  });

  it("caps long bodies with an ellipsis", () => {
    const preview = buildHeadPreview("x".repeat(200));
    assert.equal(preview.length, 141);
    assert.ok(preview.endsWith("…"));
  });
});

function build({
  active = new Map(),
  heads = new Map(),
  summaries = new Map(),
  unread = new Map(),
} = {}) {
  return buildThreadAttentionRows({
    activeSinceByThread: active,
    getHeadMessage: (id) => heads.get(id),
    getThreadSummary: (id) => summaries.get(id),
    threadUnreadCounts: unread,
  });
}

describe("buildThreadAttentionRows", () => {
  it("returns empty for no unread and no active threads", () => {
    assert.deepEqual(build(), []);
  });

  it("drops threads whose unread count is zero", () => {
    const rows = build({ unread: new Map([["t1", 0]]) });
    assert.deepEqual(rows, []);
  });

  it("merges a thread that is both unread and active into one row", () => {
    const rows = build({
      active: new Map([["t1", 1_000]]),
      unread: new Map([["t1", 3]]),
    });
    assert.equal(rows.length, 1);
    assert.equal(rows[0].threadHeadId, "t1");
    assert.equal(rows[0].unreadCount, 3);
    assert.equal(rows[0].activeSince, 1_000);
  });

  it("sorts active rows first, newest activity on top", () => {
    const rows = build({
      active: new Map([
        ["t1", 1_000],
        ["t2", 5_000],
      ]),
      summaries: new Map([["t3", { descendantCount: 2, lastReplyAt: 99 }]]),
      unread: new Map([["t3", 1]]),
    });
    assert.deepEqual(
      rows.map((row) => row.threadHeadId),
      ["t2", "t1", "t3"],
    );
  });

  it("sorts unread-only rows by reply recency, newest first", () => {
    const rows = build({
      summaries: new Map([
        ["t1", { descendantCount: 1, lastReplyAt: 10 }],
        ["t2", { descendantCount: 1, lastReplyAt: 30 }],
        ["t3", { descendantCount: 1, lastReplyAt: 20 }],
      ]),
      unread: new Map([
        ["t1", 1],
        ["t2", 1],
        ["t3", 1],
      ]),
    });
    assert.deepEqual(
      rows.map((row) => row.threadHeadId),
      ["t2", "t3", "t1"],
    );
  });

  it("breaks recency ties deterministically by thread id", () => {
    const rows = build({
      unread: new Map([
        ["b", 1],
        ["a", 1],
      ]),
    });
    assert.deepEqual(
      rows.map((row) => row.threadHeadId),
      ["a", "b"],
    );
  });

  it("carries author, preview, and reply count when the head is loaded", () => {
    const rows = build({
      heads: new Map([["t1", { author: "Bart", body: "hi\nthere" }]]),
      summaries: new Map([["t1", { descendantCount: 7, lastReplyAt: 5 }]]),
      unread: new Map([["t1", 2]]),
    });
    assert.equal(rows[0].headAuthor, "Bart");
    assert.equal(rows[0].headPreview, "hi there");
    assert.equal(rows[0].replyCount, 7);
  });

  it("degrades gracefully when the head message is not loaded", () => {
    const rows = build({ unread: new Map([["t1", 1]]) });
    assert.equal(rows[0].headAuthor, null);
    assert.equal(rows[0].headPreview, null);
    assert.equal(rows[0].replyCount, 0);
  });
});

describe("areThreadAttentionRowsEqual", () => {
  const row = {
    threadHeadId: "t1",
    headAuthor: "Bart",
    headPreview: "hi",
    replyCount: 1,
    unreadCount: 2,
    activeSince: null,
  };

  it("treats field-identical arrays as equal", () => {
    assert.ok(areThreadAttentionRowsEqual([row], [{ ...row }]));
  });

  it("detects a changed field", () => {
    assert.ok(
      !areThreadAttentionRowsEqual([row], [{ ...row, unreadCount: 3 }]),
    );
  });

  it("detects length changes", () => {
    assert.ok(!areThreadAttentionRowsEqual([row], []));
  });
});

describe("totalUnreadCount", () => {
  it("sums unread across rows", () => {
    const base = {
      threadHeadId: "t",
      headAuthor: null,
      headPreview: null,
      replyCount: 0,
      activeSince: null,
    };
    assert.equal(
      totalUnreadCount([
        { ...base, threadHeadId: "t1", unreadCount: 2 },
        { ...base, threadHeadId: "t2", unreadCount: 0 },
        { ...base, threadHeadId: "t3", unreadCount: 5 },
      ]),
      7,
    );
  });
});
