import assert from "node:assert/strict";
import test from "node:test";

import {
  canReadMessageEmbedSource,
  extractMessageEmbedLinks,
  isMatchingMessageEmbedEvent,
  messageEmbedExcerpt,
} from "./messageEmbed.ts";

const link = "buzz://message?channel=channel-1&id=event-1";

test("extractMessageEmbedLinks extracts and deduplicates bare links", () => {
  assert.deepEqual(extractMessageEmbedLinks(`See ${link}. Again: ${link}`), [
    {
      channelId: "channel-1",
      href: link,
      messageId: "event-1",
      threadRootId: null,
    },
  ]);
});

test("extractMessageEmbedLinks skips labeled and code links", () => {
  assert.deepEqual(
    extractMessageEmbedLinks(
      `[context](${link}) \`${link}\`\n\`\`\`\n${link}\n\`\`\``,
    ),
    [],
  );
});

test("canReadMessageEmbedSource allows joined and open channels only", () => {
  assert.equal(canReadMessageEmbedSource(undefined), false);
  assert.equal(
    canReadMessageEmbedSource({ isMember: false, visibility: "private" }),
    false,
  );
  assert.equal(
    canReadMessageEmbedSource({ isMember: true, visibility: "private" }),
    true,
  );
  assert.equal(
    canReadMessageEmbedSource({ isMember: false, visibility: "open" }),
    true,
  );
});

test("isMatchingMessageEmbedEvent requires both requested id and channel h tag", () => {
  const event = {
    id: "event-1",
    pubkey: "author",
    created_at: 1,
    kind: 9,
    tags: [["h", "channel-1"]],
    content: "secret",
    sig: "sig",
  };
  assert.equal(
    isMatchingMessageEmbedEvent(event, extractMessageEmbedLinks(link)[0]),
    true,
  );
  assert.equal(
    isMatchingMessageEmbedEvent(
      { ...event, id: "other" },
      extractMessageEmbedLinks(link)[0],
    ),
    false,
  );
  assert.equal(
    isMatchingMessageEmbedEvent(
      { ...event, tags: [["h", "private-other"]] },
      extractMessageEmbedLinks(link)[0],
    ),
    false,
  );
});

test("messageEmbedExcerpt normalizes whitespace without truncating source text", () => {
  assert.equal(messageEmbedExcerpt("hello\n\n  world"), "hello world");
  assert.equal(messageEmbedExcerpt("x".repeat(500)).length, 500);
});
