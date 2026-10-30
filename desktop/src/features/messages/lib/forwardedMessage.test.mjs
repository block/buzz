import assert from "node:assert/strict";
import test from "node:test";

import {
  buildForwardPayload,
  buildForwardedFromTag,
  canForwardMessage,
  forwardPreview,
  forwardProvenanceFor,
  forwardedFromAttribution,
  FORWARDED_FROM_TAG,
  getForwardedFromReference,
} from "./forwardedMessage.ts";

const KIND_STREAM_MESSAGE = 9;
const SOURCE_EVENT = "a".repeat(64);
const AUTHOR = "b".repeat(64);
const MENTION = "c".repeat(64);
const THREAD_ROOT = "d".repeat(64);
const OTHER_EVENT = "e".repeat(64);
const SOURCE_CHANNEL = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const OTHER_CHANNEL = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

function message(overrides = {}) {
  return {
    id: SOURCE_EVENT,
    pubkey: AUTHOR,
    body: "ship it",
    kind: KIND_STREAM_MESSAGE,
    depth: 0,
    author: "Alice",
    createdAt: 0,
    time: "",
    tags: [["h", SOURCE_CHANNEL]],
    ...overrides,
  };
}

test("a forwarded-from tag round-trips through build and parse", () => {
  const reference = {
    sourceEventId: SOURCE_EVENT,
    sourceChannelId: SOURCE_CHANNEL,
    authorPubkey: AUTHOR,
    threadRootId: THREAD_ROOT,
  };

  assert.deepEqual(buildForwardedFromTag(reference), [
    FORWARDED_FROM_TAG,
    SOURCE_EVENT,
    SOURCE_CHANNEL,
    AUTHOR,
    THREAD_ROOT,
  ]);
  assert.deepEqual(
    getForwardedFromReference([
      ["h", OTHER_CHANNEL],
      buildForwardedFromTag(reference),
    ]),
    reference,
  );
});

test("building a tag normalizes case and whitespace the renderer may carry", () => {
  assert.deepEqual(
    buildForwardedFromTag({
      sourceEventId: `  ${SOURCE_EVENT.toUpperCase()}  `,
      sourceChannelId: SOURCE_CHANNEL.toUpperCase(),
      authorPubkey: AUTHOR.toUpperCase(),
      threadRootId: null,
    }),
    [FORWARDED_FROM_TAG, SOURCE_EVENT, SOURCE_CHANNEL, AUTHOR],
  );
});

test("building a tag refuses an incomplete source reference", () => {
  for (const broken of [
    { sourceEventId: "not-an-event-id" },
    { sourceChannelId: "not-a-uuid" },
    { authorPubkey: "" },
    { threadRootId: "nope" },
  ]) {
    assert.throws(
      () =>
        buildForwardedFromTag({
          sourceEventId: SOURCE_EVENT,
          sourceChannelId: SOURCE_CHANNEL,
          authorPubkey: AUTHOR,
          threadRootId: null,
          ...broken,
        }),
      /complete source reference/,
      `expected ${JSON.stringify(broken)} to be rejected`,
    );
  }
});

test("a malformed or forged provenance tag reads as no provenance", () => {
  const cases = [
    [["h", SOURCE_CHANNEL, AUTHOR, SOURCE_EVENT]],
    [["p", AUTHOR, SOURCE_CHANNEL, SOURCE_EVENT]],
    [[FORWARDED_FROM_TAG, SOURCE_EVENT, SOURCE_CHANNEL]],
    [
      [
        FORWARDED_FROM_TAG,
        SOURCE_EVENT,
        SOURCE_CHANNEL,
        AUTHOR,
        THREAD_ROOT,
        "x",
      ],
    ],
    [[FORWARDED_FROM_TAG, "short", SOURCE_CHANNEL, AUTHOR]],
    [[FORWARDED_FROM_TAG, SOURCE_EVENT, "not-a-uuid", AUTHOR]],
    [[FORWARDED_FROM_TAG, SOURCE_EVENT, SOURCE_CHANNEL, "not-a-pubkey"]],
    [[FORWARDED_FROM_TAG, SOURCE_EVENT, SOURCE_CHANNEL, AUTHOR, "not-a-root"]],
  ];

  for (const tags of cases) {
    assert.equal(
      getForwardedFromReference(tags),
      null,
      `expected ${JSON.stringify(tags)} to read as no provenance`,
    );
  }
  assert.equal(getForwardedFromReference(undefined), null);
});

test("provenance names the message's own channel, thread root and author", () => {
  assert.deepEqual(
    forwardProvenanceFor(
      message({
        tags: [
          ["h", SOURCE_CHANNEL],
          ["e", THREAD_ROOT, "", "root"],
          ["e", OTHER_EVENT, "", "reply"],
        ],
      }),
      SOURCE_CHANNEL,
    ),
    {
      sourceEventId: SOURCE_EVENT,
      sourceChannelId: SOURCE_CHANNEL,
      authorPubkey: AUTHOR,
      threadRootId: THREAD_ROOT,
    },
  );
});

test("forwarding a forward keeps pointing at the first original", () => {
  const original = {
    sourceEventId: OTHER_EVENT,
    sourceChannelId: OTHER_CHANNEL,
    authorPubkey: MENTION,
    threadRootId: null,
  };
  // The copy sitting in SOURCE_CHANNEL was itself forwarded from elsewhere;
  // forwarding it again must credit the original author, not the forwarder.
  assert.deepEqual(
    forwardProvenanceFor(
      message({
        tags: [["h", SOURCE_CHANNEL], buildForwardedFromTag(original)],
      }),
      SOURCE_CHANNEL,
    ),
    original,
  );
});

test("an edited forward starts a new provenance chain", () => {
  const editedCopy = message({
    id: OTHER_EVENT,
    pubkey: MENTION,
    edited: true,
    tags: [
      ["h", SOURCE_CHANNEL],
      buildForwardedFromTag({
        sourceEventId: SOURCE_EVENT,
        sourceChannelId: SOURCE_CHANNEL,
        authorPubkey: AUTHOR,
        threadRootId: null,
      }),
    ],
  });

  assert.deepEqual(forwardProvenanceFor(editedCopy, SOURCE_CHANNEL), {
    sourceEventId: OTHER_EVENT,
    sourceChannelId: SOURCE_CHANNEL,
    authorPubkey: MENTION,
    threadRootId: null,
  });
});

test("a message that cannot be attributed cannot be forwarded", () => {
  assert.equal(forwardProvenanceFor(message(), null), null);
  assert.equal(
    forwardProvenanceFor(message({ pubkey: undefined }), SOURCE_CHANNEL),
    null,
  );
  assert.equal(
    forwardProvenanceFor(message({ id: "optimistic-1" }), SOURCE_CHANNEL),
    null,
  );

  assert.equal(canForwardMessage(message(), SOURCE_CHANNEL), true);
  assert.equal(
    canForwardMessage(message({ pending: true }), SOURCE_CHANNEL),
    false,
  );
  assert.equal(
    canForwardMessage(message({ kind: 40099 }), SOURCE_CHANNEL),
    false,
  );
  assert.equal(canForwardMessage(message(), null), false);
});

test("a forward copies the body and its content tags but not its audience", () => {
  const imeta = ["imeta", "url https://relay.example/a.png"];
  const emoji = ["emoji", "party", "https://relay.example/party.png"];
  const mention = ["mention", MENTION];

  const payload = buildForwardPayload(
    message({
      tags: [
        ["h", SOURCE_CHANNEL],
        ["e", OTHER_EVENT, "", "reply"],
        ["p", AUTHOR],
        ["p", MENTION],
        imeta,
        emoji,
        mention,
        ["client", "source-only-marker"],
      ],
    }),
    SOURCE_CHANNEL,
  );

  assert.equal(payload.content, "ship it");
  // `mention` identifies so the chip still renders; `p` notifies, and the
  // people paged in the source channel did not ask to be paged here.
  assert.deepEqual(payload.contentTags, [imeta, emoji, mention]);
  assert.equal(
    payload.contentTags.some((tag) => tag[0] === "p"),
    false,
  );
  assert.deepEqual(payload.forwardedFrom, {
    sourceEventId: SOURCE_EVENT,
    sourceChannelId: SOURCE_CHANNEL,
    authorPubkey: AUTHOR,
    threadRootId: OTHER_EVENT,
  });
});

test("an unattributable message yields no forward payload", () => {
  assert.equal(buildForwardPayload(message(), undefined), null);
});

test("the confirmation preview collapses whitespace and clips long bodies", () => {
  assert.equal(forwardPreview("  ship\n\n  it  "), "ship it");
  assert.equal(forwardPreview(undefined), "");

  const long = forwardPreview("word ".repeat(60));
  assert.equal(long.endsWith("…"), true);
  assert.equal(Array.from(long).length <= 140, true);
});

test("an edited copy stops claiming whose words it carries", () => {
  // The forwarder owns the copy and can rewrite it; nothing re-signs the
  // provenance tag when they do, so the attribution must not keep presenting
  // the edited text as the original author's.
  assert.equal(forwardedFromAttribution("Alice"), "Forwarded from Alice:");
  assert.equal(
    forwardedFromAttribution("Alice", false),
    "Forwarded from Alice:",
  );

  const edited = forwardedFromAttribution("Alice", true);
  assert.equal(edited, "Forwarded and edited:");
  assert.equal(edited.includes("Alice"), false);
});
