import assert from "node:assert/strict";
import test from "node:test";

import { formatFullDateTime } from "./dateFormatters.ts";
import {
  buildThreadTranscript,
  replaceMediaEmbedsWithPlaceholders,
  serializeThreadMessages,
} from "./threadTranscript.ts";

const ROOT_AT = 1_760_000_000;

function makeMessage(overrides = {}) {
  return {
    author: "alice",
    body: "hello",
    createdAt: ROOT_AT,
    depth: 0,
    id: `msg-${Math.random().toString(36).slice(2)}`,
    time: "2:34 PM",
    ...overrides,
  };
}

test("serializes one block per message with author, timestamp, and body", () => {
  const transcript = serializeThreadMessages([
    makeMessage({ author: "alice", body: "root question", createdAt: ROOT_AT }),
    makeMessage({
      author: "helper-agent",
      body: "an answer",
      createdAt: ROOT_AT + 60,
    }),
  ]);

  assert.equal(
    transcript,
    [
      `alice — ${formatFullDateTime(ROOT_AT)}`,
      "root question",
      "",
      `helper-agent — ${formatFullDateTime(ROOT_AT + 60)}`,
      "an answer",
    ].join("\n"),
  );
});

test("preserves the given order (root first)", () => {
  const transcript = serializeThreadMessages([
    makeMessage({ author: "root", body: "first" }),
    makeMessage({ author: "reply-1", body: "second" }),
    makeMessage({ author: "reply-2", body: "third" }),
  ]);

  const authorOrder = transcript
    .split("\n")
    .filter((line) => line.includes(" — "))
    .map((line) => line.split(" — ")[0]);
  assert.deepEqual(authorOrder, ["root", "reply-1", "reply-2"]);
});

test("keeps multi-paragraph bodies intact inside a block", () => {
  const body = "first paragraph\n\nsecond paragraph\nwith a wrapped line";
  const transcript = serializeThreadMessages([makeMessage({ body })]);

  assert.equal(transcript, `alice — ${formatFullDateTime(ROOT_AT)}\n${body}`);
});

test("marks edited messages on the header line", () => {
  const transcript = serializeThreadMessages([
    makeMessage({ body: "fixed typo", edited: true }),
  ]);

  assert.equal(
    transcript,
    `alice — ${formatFullDateTime(ROOT_AT)} (edited)\nfixed typo`,
  );
});

test("replaces image embeds with a filename placeholder from imeta", () => {
  const url = "https://relay.example.com/media/abc123.png";
  const text = replaceMediaEmbedsWithPlaceholders(
    `look at this\n![image](${url})`,
    [["imeta", `url ${url}`, "m image/png", "filename screenshot.png"]],
  );

  assert.equal(text, "look at this\n[image: screenshot.png]");
});

test("replaces media embeds without imeta with bare placeholders", () => {
  const text = replaceMediaEmbedsWithPlaceholders(
    "before\n![image](https://x.test/a.png)\n||![video](https://x.test/b.mp4)||",
  );

  assert.equal(text, "before\n[image]\n[video]");
});

test("collapses imeta file links but leaves user links alone", () => {
  const fileUrl = "https://relay.example.com/media/deadbeef";
  const text = replaceMediaEmbedsWithPlaceholders(
    `see [notes.pdf](${fileUrl}) and [docs](https://example.com)`,
    [["imeta", `url ${fileUrl}`, "m application/pdf", "filename notes.pdf"]],
  );

  assert.equal(text, "see [file: notes.pdf] and [docs](https://example.com)");
});

test("attachment-only message keeps its header with the placeholder body", () => {
  const url = "https://relay.example.com/media/pic";
  const transcript = serializeThreadMessages([
    makeMessage({
      body: `\n![image](${url})`,
      tags: [["imeta", `url ${url}`, "m image/jpeg", "filename pic.jpg"]],
    }),
  ]);

  assert.equal(
    transcript,
    `alice — ${formatFullDateTime(ROOT_AT)}\n[image: pic.jpg]`,
  );
});

test("empty body serializes to just the header line", () => {
  const transcript = serializeThreadMessages([
    makeMessage({ body: "" }),
    makeMessage({ author: "bob", body: "  \n ", createdAt: ROOT_AT + 5 }),
  ]);

  assert.equal(
    transcript,
    [
      `alice — ${formatFullDateTime(ROOT_AT)}`,
      "",
      `bob — ${formatFullDateTime(ROOT_AT + 5)}`,
    ].join("\n"),
  );
});

test("serializing no messages yields an empty string", () => {
  assert.equal(serializeThreadMessages([]), "");
});

test("buildThreadTranscript places the head before the visible entries", () => {
  const head = makeMessage({ author: "root", body: "thread head" });
  const entries = [
    {
      message: makeMessage({ author: "alice", body: "reply A" }),
      summary: null,
    },
    { message: makeMessage({ author: "bob", body: "reply B" }), summary: null },
  ];

  const transcript = buildThreadTranscript(head, entries);
  const authorOrder = transcript
    .split("\n")
    .filter((line) => line.includes(" — "))
    .map((line) => line.split(" — ")[0]);
  assert.deepEqual(authorOrder, ["root", "alice", "bob"]);
});
