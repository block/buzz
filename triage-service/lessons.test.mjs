import assert from "node:assert/strict";
import test from "node:test";

import { buildLessons, renderCorrections } from "./lessons.mjs";

test("buildLessons weights events, authors, channels, and threads", () => {
  const lessons = buildLessons([
    {
      userAction: "dismissed",
      eventId: "e1",
      authorPubkey: "vlad",
      channelId: "c1",
      threadRootId: "t1",
      preview: "lunch",
    },
    {
      userAction: "done",
      eventId: "e2",
      authorPubkey: "vlad",
      channelId: "c1",
      preview: "migration",
    },
    { userAction: "snoozed", authorPubkey: "vlad", preview: "ignored action" },
  ]);

  assert.equal(lessons.events.get("e1"), -1);
  assert.equal(lessons.authors.get("vlad"), 0);
  assert.equal(lessons.channels.get("c1"), 0);
  assert.equal(lessons.threads.get("t1"), -1);
  assert.equal(lessons.examples.length, 2);
});

test("written reasons are picked for the prompt ahead of bare dismissals", () => {
  const feedback = Array.from({ length: 15 }, (_, index) => ({
    userAction: "dismissed",
    preview: `bare ${index}`,
  }));
  feedback.push({
    userAction: "dismissed",
    preview: "lunch thread",
    note: "social channel, never actionable",
  });

  const lessons = buildLessons(feedback);
  assert.equal(lessons.examples[0].preview, "lunch thread");
  assert.equal(lessons.examples.length, 12);
});

test("renderCorrections passes a stated reason through verbatim", () => {
  const rendered = renderCorrections({
    examples: [
      {
        preview: "lunch thread",
        userAction: "dismissed",
        note: "social channel, never actionable",
      },
      { preview: "migration", userAction: "done", note: null },
    ],
  });

  assert.match(rendered, /standing instruction/);
  assert.match(
    rendered,
    /"lunch thread" -> dismissed, because: social channel, never actionable/,
  );
  assert.match(rendered, /"migration" -> done\n/);
});

test("renderCorrections is empty when there is nothing to teach", () => {
  assert.equal(renderCorrections({ examples: [] }), "");
  assert.equal(renderCorrections(undefined), "");
});
