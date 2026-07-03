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

test("estimateRowHeight: bare URL line adds a preview card", () => {
  const withUrl = estimateRowHeight(msg({ body: "https://example.com/x" }));
  const withoutUrl = estimateRowHeight(msg({ body: "example" }));
  assert.ok(withUrl > withoutUrl + 50, `url ${withUrl} vs ${withoutUrl}`);
});

test("timelineRowReserveStyle: message item yields containIntrinsicSize", () => {
  const style = timelineRowReserveStyle({
    kind: "message",
    key: "k",
    entry: { message: msg({ body: "hi" }), summary: null },
  });
  assert.match(String(style.containIntrinsicSize), /^auto \d+px$/);
});

test("timelineRowReserveStyle: divider is short fixed height", () => {
  const style = timelineRowReserveStyle({
    kind: "day-divider",
    key: "k",
    headingTimestamp: 0,
  });
  assert.equal(style.containIntrinsicSize, "auto 32px");
});
