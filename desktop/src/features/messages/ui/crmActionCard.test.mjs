import assert from "node:assert/strict";
import test from "node:test";

import { extractCrmRedditDraft, parseCrmActionCard } from "./crmActionCardParser.ts";

const marker = "crm-action:v1:8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a:reddit_mark_posted:2026-07-26T20:15:00+00:00";

test("parses a strict CRM action marker and removes it from rendered content", () => {
  assert.deepEqual(parseCrmActionCard(`Mark Reddit draft as posted.\n${marker}`), {
    actionId: "8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a",
    actionType: "reddit_mark_posted",
    expiresAt: "2026-07-26T20:15:00+00:00",
    content: "Mark Reddit draft as posted.",
  });
});

test("parses the supported lead categorization action marker", () => {
  assert.equal(
    parseCrmActionCard("Categorize lead as: Interested.\ncrm-action:v1:8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a:lead_categorize:2026-07-26T20:15:00+00:00")?.actionType,
    "lead_categorize",
  );
});

test("parses the supported outreach approval action marker", () => {
  assert.equal(
    parseCrmActionCard("Review outreach.\ncrm-action:v1:8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a:outreach_approve:2026-07-26T20:15:00+00:00")?.actionType,
    "outreach_approve",
  );
});

test("preserves read-only Reddit review content ahead of the action controls", () => {
  const review = [
    "Reddit post: Storage question",
    "Subreddit: r/selfstorage",
    "Open thread: https://www.reddit.com/r/selfstorage/comments/1",
    "",
    "Draft to copy manually:",
    "```",
    "Safe reply text",
    "```",
    "",
    "Mark Reddit draft as posted.",
    marker,
  ].join("\n");

  const card = parseCrmActionCard(review);
  assert.equal(card?.actionType, "reddit_mark_posted");
  assert.match(card?.content ?? "", /Open thread: https:\/\/www\.reddit\.com/);
  assert.match(card?.content ?? "", /```\nSafe reply text\n```/);
  assert.doesNotMatch(card?.content ?? "", /crm-action:v1:/);
  assert.equal(extractCrmRedditDraft(card?.content ?? ""), "Safe reply text");
});

test("does not expose a copy value without the explicit Reddit draft delimiter", () => {
  assert.equal(extractCrmRedditDraft("A fenced block is not necessarily a Reddit draft.\n```\nIgnore me\n```"), null);
});

test("preserves an internal code fence when the CRM uses a longer outer fence", () => {
  const review = [
    "Draft to copy manually:",
    "````",
    "Use this example:",
    "```text",
    "safe content",
    "```",
    "Then continue the reply.",
    "````",
  ].join("\n");

  assert.equal(
    extractCrmRedditDraft(review),
    "Use this example:\n```text\nsafe content\n```\nThen continue the reply.",
  );
});

test("uses only the terminal CRM marker when review content contains a marker-like line", () => {
  const injected = "crm-action:v1:00000000-0000-4000-8000-000000000000:lead_categorize:2026-07-26T20:15:00+00:00";
  const card = parseCrmActionCard(`Draft to copy manually:\n${injected}\n\nMark Reddit draft as posted.\n${marker}`);

  assert.equal(card?.actionId, "8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a");
  assert.equal(card?.actionType, "reddit_mark_posted");
  assert.ok((card?.content ?? "").includes(injected));
});

test("rejects malformed or unsupported markers", () => {
  assert.equal(parseCrmActionCard("crm-action:v1:not-a-uuid:reddit_mark_posted:2026-07-26T20:15:00+00:00"), null);
  assert.equal(parseCrmActionCard("crm-action:v1:8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a:delete_company:2026-07-26T20:15:00+00:00"), null);
  assert.equal(parseCrmActionCard("crm-action:v1:8ca5bd14-00d4-45cc-88ec-4bb1609e7d4a:reddit_mark_posted:not-a-date"), null);
});
