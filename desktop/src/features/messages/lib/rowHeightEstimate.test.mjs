import assert from "node:assert/strict";
import { test } from "node:test";

import {
  estimateRowHeight,
  timelineRowReserveStyle,
} from "./rowHeightEstimate.ts";

function msg(over = {}) {
  return {
    id: "m1",
    createdAt: 0,
    author: "a",
    time: "now",
    body: "",
    depth: 0,
    ...over,
  };
}

test("estimateRowHeight: short text is near the floor", () => {
  const h = estimateRowHeight(msg({ body: "hello" }));
  assert.ok(h >= 60 && h < 120, `expected small, got ${h}`);
});

test("estimateRowHeight: many lines reserve more", () => {
  const tall = estimateRowHeight(
    msg({ body: Array.from({ length: 20 }, (_, i) => `line ${i}`).join("\n") }),
  );
  const short = estimateRowHeight(msg({ body: "one line" }));
  assert.ok(tall > short + 200, `tall ${tall} vs short ${short}`);
});

test("estimateRowHeight: fenced code adds height by line", () => {
  const withCode = estimateRowHeight(
    msg({ body: "see:\n```\na\nb\nc\nd\ne\n```" }),
  );
  const withoutCode = estimateRowHeight(msg({ body: "see:" }));
  assert.ok(withCode > withoutCode + 80, `code ${withCode} vs ${withoutCode}`);
});

test("estimateRowHeight: imeta image with dim reserves bounded media height", () => {
  const tagged = estimateRowHeight(
    msg({
      body: "shot",
      tags: [["imeta", "url http://x/a.png", "m image/png", "dim 320x240"]],
    }),
  );
  // 320x240 -> 4:3, width-bound 384/(4/3)=288 capped at 256, plus chrome+text.
  assert.ok(tagged >= 256 && tagged <= 360, `got ${tagged}`);
});

test("estimateRowHeight: dim-less imeta reserves the full media box", () => {
  const noDim = estimateRowHeight(
    msg({
      body: "shot",
      tags: [["imeta", "url http://x/a.png", "m image/png"]],
    }),
  );
  assert.ok(noDim >= 256, `got ${noDim}`);
});

test("estimateRowHeight: markdown image with NO imeta reserves media box", () => {
  const h = estimateRowHeight(
    msg({ body: "look\n![](https://example.com/pic.png)" }),
  );
  assert.ok(h >= 256, `expected full media reserve, got ${h}`);
});

test("estimateRowHeight: bare media URL line reserves media box, not a card", () => {
  const h = estimateRowHeight(msg({ body: "https://example.com/clip.mp4" }));
  assert.ok(h >= 256, `expected media reserve, got ${h}`);
});

test("estimateRowHeight: imeta dim is not double-counted with its body url", () => {
  const url = "https://x/a.png";
  const both = estimateRowHeight(
    msg({
      body: `![](${url})`,
      tags: [["imeta", `url ${url}`, "m image/png", "dim 320x240"]],
    }),
  );
  // One media reserve (~256 capped) + chrome, not two.
  assert.ok(both < 400, `expected single media reserve, got ${both}`);
});

test("estimateRowHeight: unsupported bare URL line does not add a preview card", () => {
  const withUrl = estimateRowHeight(msg({ body: "https://example.com/x" }));
  const withoutUrl = estimateRowHeight(msg({ body: "example" }));
  assert.ok(withUrl < withoutUrl + 25, `url ${withUrl} vs ${withoutUrl}`);
});

test("estimateRowHeight: supported GitHub URL line adds a preview card", () => {
  const withUrl = estimateRowHeight(
    msg({ body: "https://github.com/block/buzz/pull/1641" }),
  );
  const withoutUrl = estimateRowHeight(msg({ body: "example" }));
  assert.ok(withUrl > withoutUrl + 50, `url ${withUrl} vs ${withoutUrl}`);
});

test("estimateRowHeight: supported Linear and Google URLs add preview cards", () => {
  const base = estimateRowHeight(msg({ body: "example" }));
  const linear = estimateRowHeight(
    msg({ body: "https://linear.app/block/issue/BUZZ-123/fix-scroll" }),
  );
  const google = estimateRowHeight(
    msg({ body: "https://docs.google.com/document/d/abc123/edit" }),
  );
  assert.ok(linear > base + 50, `linear ${linear} vs ${base}`);
  assert.ok(google > base + 50, `google ${google} vs ${base}`);
});

test("estimateRowHeight: column width option changes prose wrapping", () => {
  const body = "x".repeat(160);
  const narrow = estimateRowHeight(msg({ body }), { columnWidthPx: 320 });
  const fallback = estimateRowHeight(msg({ body }));
  assert.ok(narrow > fallback + 20, `narrow ${narrow} vs fallback ${fallback}`);
});

test("estimateRowHeight: markdown structures reserve extra chrome", () => {
  const table = estimateRowHeight(
    msg({ body: "| A | B |\n| - | - |\n| 1 | 2 |" }),
  );
  const plain = estimateRowHeight(msg({ body: "A B\n- -\n1 2" }));
  assert.ok(table > plain + 20, `table ${table} vs plain ${plain}`);
});

test("timelineRowReserveStyle: message item yields containIntrinsicSize", () => {
  const style = timelineRowReserveStyle({
    kind: "message",
    key: "k",
    entry: { message: msg({ body: "hi" }), summary: null },
    isContinuation: false,
    isFollowedByContinuation: false,
  });
  assert.match(String(style.containIntrinsicSize), /^auto \d+px$/);
});

test("timelineRowReserveStyle: summary rows add summary chrome", () => {
  const item = {
    kind: "message",
    key: "k",
    entry: {
      message: msg({ body: "hi" }),
      summary: {
        threadHeadId: "m1",
        replyCount: 3,
        lastReplyAt: 1,
        participants: [],
      },
    },
    isContinuation: false,
    isFollowedByContinuation: false,
  };
  const withSummary = Number.parseInt(
    String(timelineRowReserveStyle(item).containIntrinsicSize).match(
      /auto (\d+)px/,
    )?.[1] ?? "0",
    10,
  );
  const withoutSummary = Number.parseInt(
    String(
      timelineRowReserveStyle({
        ...item,
        entry: { ...item.entry, summary: null },
      }).containIntrinsicSize,
    ).match(/auto (\d+)px/)?.[1] ?? "0",
    10,
  );
  assert.ok(
    withSummary > withoutSummary + 25,
    `${withSummary} vs ${withoutSummary}`,
  );
});

test("timelineRowReserveStyle: divider is short fixed height", () => {
  const style = timelineRowReserveStyle({
    kind: "day-divider",
    key: "k",
    headingTimestamp: 0,
  });
  assert.equal(style.containIntrinsicSize, "auto 32px");
});
