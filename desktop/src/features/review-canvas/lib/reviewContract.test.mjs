import assert from "node:assert/strict";
import { test } from "node:test";

import {
  bindFeedbackToReview,
  buildFeedbackArtifact,
  isWorkspaceRelativePath,
  normalizeFeedbackRequest,
  parseReviewFeedback,
  parseReviewFeedbackList,
  parseReviewNotification,
  parseReviewRevision,
  validateReviewBlock,
} from "./reviewContract.ts";
import {
  ADAPTER,
  AGENT,
  ARTIFACT_ID,
  CHANNEL_ID,
  FEEDBACK_EVENT,
  FEEDBACK_ID,
  NOTIFICATION_ID,
  ORIGIN_EVENT,
  REVIEWER,
  REVISION_EVENT,
  REVISION_TWO_EVENT,
  THREE_BLOCK_HTML,
  feedbackEvent,
  notificationMessage,
  reviewEvent,
  sha256,
} from "./reviewFixtures.mjs";

// ── Notification recognition ────────────────────────────────────────────────

test("a kind-9 message with the shared custom tags is a review notification", () => {
  const notification = parseReviewNotification(notificationMessage());
  assert.deepEqual(notification, {
    messageId: NOTIFICATION_ID,
    artifactId: ARTIFACT_ID,
    revisionEventId: REVISION_EVENT,
    executiveAgentPubkey: AGENT,
    payloadDigest: sha256(THREE_BLOCK_HTML),
    authorPubkey: ADAPTER,
    threadRootId: ORIGIN_EVENT,
  });
});

test("a top-level notification anchors its own thread", () => {
  const notification = parseReviewNotification(
    notificationMessage({ parentId: null, rootId: null }),
  );
  assert.equal(notification?.threadRootId, NOTIFICATION_ID);
});

test("only well-formed, unambiguous kind-9 notifications are recognized", () => {
  const base = notificationMessage();
  const without = (name) => ({
    ...base,
    tags: base.tags.filter((tag) => tag[0] !== name),
  });
  const replacing = (name, tag) => ({
    ...base,
    tags: base.tags.map((existing) => (existing[0] === name ? tag : existing)),
  });
  const rejected = {
    "wrong kind": { ...base, kind: 40002 },
    "no kind": { ...base, kind: undefined },
    "no tags": { ...base, tags: undefined },
    "missing artifact": without("artifact"),
    "missing artifact_type": without("artifact_type"),
    "missing agent": without("agent"),
    "missing digest": without("x"),
    "other artifact type": replacing("artifact_type", [
      "artifact_type",
      "buzz.task",
    ]),
    "uppercase artifact uuid": replacing("artifact", [
      "artifact",
      ARTIFACT_ID.toUpperCase(),
      REVISION_EVENT,
    ]),
    "short revision id": replacing("artifact", [
      "artifact",
      ARTIFACT_ID,
      "abc",
    ]),
    "extra artifact element": replacing("artifact", [
      "artifact",
      ARTIFACT_ID,
      REVISION_EVENT,
      "extra",
    ]),
    "non-hex agent": replacing("agent", ["agent", "npub1notahexkey"]),
    "duplicate artifact tag": {
      ...base,
      tags: [...base.tags, ["artifact", ARTIFACT_ID, REVISION_TWO_EVENT]],
    },
    "duplicate digest tag": {
      ...base,
      tags: [...base.tags, ["x", "9".repeat(64)]],
    },
  };
  for (const [name, message] of Object.entries(rejected)) {
    assert.equal(parseReviewNotification(message), null, name);
  }
});

test("an ordinary HTML attachment message is not a review notification", () => {
  const html = notificationMessage({
    tags: [
      ["h", CHANNEL_ID],
      [
        "imeta",
        `url https://relay.example/media/${"a".repeat(64)}.html`,
        "m text/html",
        `x ${"a".repeat(64)}`,
        "size 12",
      ],
    ],
  });
  assert.equal(parseReviewNotification(html), null);
});

// ── Review revision ─────────────────────────────────────────────────────────

test("a well-formed review revision parses with its attribution and document", () => {
  const revision = parseReviewRevision(reviewEvent());
  assert.equal(revision.artifactId, ARTIFACT_ID);
  assert.equal(revision.channelId, CHANNEL_ID);
  assert.equal(revision.eventId, REVISION_EVENT);
  assert.equal(revision.authorPubkey, ADAPTER);
  assert.equal(revision.rootEventId, ORIGIN_EVENT);
  assert.equal(revision.prevEventId, null);
  assert.equal(revision.revision, 1);
  assert.deepEqual(revision.attribution, {
    requestedBy: REVIEWER,
    submittedBy: ADAPTER,
    originEventId: ORIGIN_EVENT,
  });
  assert.equal(
    revision.synaxisArtifact.payloadDigest,
    sha256(THREE_BLOCK_HTML),
  );
  assert.equal(revision.document.sha256, sha256(THREE_BLOCK_HTML));
  assert.deepEqual(revision.dispositions, []);
});

test("a later revision carries prev and feedback dispositions", () => {
  const revision = parseReviewRevision(
    reviewEvent({
      eventId: REVISION_TWO_EVENT,
      revision: 2,
      dispositions: [
        { feedback_revision_event_id: FEEDBACK_EVENT, status: "addressed" },
        { feedback_revision_event_id: "4".repeat(64), status: "unresolved" },
      ],
    }),
  );
  assert.equal(revision.prevEventId, REVISION_EVENT);
  assert.deepEqual(revision.dispositions, [
    { feedbackRevisionEventId: FEEDBACK_EVENT, status: "addressed" },
    { feedbackRevisionEventId: "4".repeat(64), status: "unresolved" },
  ]);
});

test("the content hash, imeta hash, and artifact digest must all agree", () => {
  const event = reviewEvent();
  const other = "9".repeat(64);

  const imetaDrift = {
    ...event,
    tags: event.tags.map((tag) =>
      tag[0] === "imeta" ? [tag[0], tag[1], tag[2], `x ${other}`, tag[4]] : tag,
    ),
  };
  assert.throws(() => parseReviewRevision(imetaDrift), /disagree/);

  const content = JSON.parse(event.content);
  content.artifact.payload_digest = other;
  assert.throws(
    () => parseReviewRevision({ ...event, content: JSON.stringify(content) }),
    /disagree/,
  );

  const blobDrift = JSON.parse(event.content);
  blobDrift.presentation.blob_sha256 = other;
  assert.throws(
    () => parseReviewRevision({ ...event, content: JSON.stringify(blobDrift) }),
    /disagree/,
  );
});

test("malformed or hostile review revisions are rejected", () => {
  const event = reviewEvent();
  const mutate = (edit) => {
    const content = JSON.parse(event.content);
    edit(content);
    return { ...event, content: JSON.stringify(content) };
  };
  const rejected = {
    "wrong kind": { ...event, kind: 9 },
    "wrong schema": mutate((c) => {
      c.schema = "synaxis.html-review/v2";
    }),
    "feedback type": {
      ...event,
      tags: event.tags.map((tag) =>
        tag[0] === "type" ? ["type", "synaxis.artifact-feedback"] : tag,
      ),
    },
    "missing attribution": mutate((c) => {
      c.attribution = undefined;
    }),
    "non-hex attribution": mutate((c) => {
      c.attribution.requested_by = "someone";
    }),
    "non-html presentation": mutate((c) => {
      c.presentation.mime_type = "image/svg+xml";
    }),
    "oversized document": mutate((c) => {
      c.presentation.byte_size = 4 * 1024 * 1024 + 1;
    }),
    "zero-byte document": mutate((c) => {
      c.presentation.byte_size = 0;
    }),
    "unknown disposition status": mutate((c) => {
      c.feedback_dispositions = [
        { feedback_revision_event_id: FEEDBACK_EVENT, status: "approved" },
      ];
    }),
    "duplicate disposition": mutate((c) => {
      c.feedback_dispositions = [
        { feedback_revision_event_id: FEEDBACK_EVENT, status: "addressed" },
        { feedback_revision_event_id: FEEDBACK_EVENT, status: "unresolved" },
      ];
    }),
    "no imeta": {
      ...event,
      tags: event.tags.filter((tag) => tag[0] !== "imeta"),
    },
    "missing root": {
      ...event,
      tags: event.tags.filter((tag) => tag[0] !== "root"),
    },
    "duplicate imeta": { ...event, tags: [...event.tags, event.tags.at(-1)] },
    "ftp attachment": {
      ...event,
      tags: event.tags.map((tag) =>
        tag[0] === "imeta"
          ? [tag[0], "url ftp://relay.example/x.html", ...tag.slice(2)]
          : tag,
      ),
    },
    "deleted artifact": {
      ...event,
      tags: event.tags.map((tag) => (tag[0] === "op" ? ["op", "delete"] : tag)),
    },
    "not json": { ...event, content: "<html>" },
    "json array": { ...event, content: "[]" },
  };
  for (const [name, candidate] of Object.entries(rejected)) {
    assert.throws(() => parseReviewRevision(candidate), undefined, name);
  }
});

// ── Feedback construction and parsing ───────────────────────────────────────

function revisionFixture() {
  return parseReviewRevision(reviewEvent());
}

test("feedback is bound to the exact reviewed revision, digest, and block", () => {
  const revision = revisionFixture();
  const template = buildFeedbackArtifact({
    feedbackArtifactId: FEEDBACK_ID,
    revision,
    block: {
      id: "checkout.primary-action",
      title: "Primary checkout action",
      sourceRef: "src/features/checkout/CheckoutActions.tsx",
    },
    request: "  Move this above the order summary.  ",
    viewport: { width: 1440.4, height: 900 },
  });
  assert.equal(template.kind, 45010);
  assert.deepEqual(template.tags, [
    ["ar", "1"],
    ["d", FEEDBACK_ID],
    ["h", CHANNEL_ID],
    ["type", "synaxis.artifact-feedback"],
    ["title", "Feedback: Primary checkout action"],
    ["op", "create"],
    ["root", ORIGIN_EVENT],
    ["target", "checkout.primary-action"],
    ["target_revision", REVISION_EVENT],
    ["synaxis_artifact", "01SYNAXISARTIFACT"],
    ["payload", sha256(THREE_BLOCK_HTML)],
  ]);
  assert.deepEqual(JSON.parse(template.content), {
    schema: "synaxis.artifact-feedback/v1",
    reviewed: {
      buzz_artifact_id: ARTIFACT_ID,
      buzz_revision_event_id: REVISION_EVENT,
      synaxis_artifact_id: "01SYNAXISARTIFACT",
      payload_digest: sha256(THREE_BLOCK_HTML),
    },
    target: {
      review_id: "checkout.primary-action",
      title: "Primary checkout action",
      source_ref: "src/features/checkout/CheckoutActions.tsx",
    },
    context: { viewport: { width: 1440, height: 900 }, interaction_state: {} },
    request: "Move this above the order summary.",
  });
  // No artifact source or HTML bytes are copied into feedback.
  assert.equal(template.content.includes("<section"), false);
});

test("feedback envelope titles stay within the relay UTF-8 byte limit", () => {
  const title = "界".repeat(200);
  const template = buildFeedbackArtifact({
    feedbackArtifactId: FEEDBACK_ID,
    revision: revisionFixture(),
    block: { id: "checkout.summary", title, sourceRef: null },
    request: "Tighten the spacing.",
    viewport: { width: 800, height: 600 },
  });
  const envelopeTitle = template.tags.find((tag) => tag[0] === "title")[1];
  assert.ok(Buffer.byteLength(envelopeTitle, "utf8") <= 512);
  assert.equal(JSON.parse(template.content).target.title, title);
});

test("a block without a source reference omits source_ref", () => {
  const template = buildFeedbackArtifact({
    feedbackArtifactId: FEEDBACK_ID,
    revision: revisionFixture(),
    block: { id: "checkout.summary", title: "Order summary", sourceRef: null },
    request: "Tighten the spacing.",
    viewport: { width: 800, height: 600 },
  });
  assert.equal("source_ref" in JSON.parse(template.content).target, false);
});

test("blank, oversized, and non-canonical feedback is refused before signing", () => {
  const input = {
    feedbackArtifactId: FEEDBACK_ID,
    revision: revisionFixture(),
    block: { id: "checkout.summary", title: "Order summary", sourceRef: null },
    request: "ok",
    viewport: { width: 800, height: 600 },
  };
  assert.throws(() => buildFeedbackArtifact({ ...input, request: "  \n " }));
  assert.throws(() =>
    buildFeedbackArtifact({ ...input, request: "é".repeat(4097) }),
  );
  assert.throws(() =>
    buildFeedbackArtifact({ ...input, feedbackArtifactId: "not-a-uuid" }),
  );
  assert.equal(
    normalizeFeedbackRequest(" line one\nline two "),
    "line one\nline two",
  );
});

test("feedback round-trips through the parser and keeps multi-line requests", () => {
  const revision = revisionFixture();
  const template = buildFeedbackArtifact({
    feedbackArtifactId: FEEDBACK_ID,
    revision,
    block: { id: "checkout.summary", title: "Order summary", sourceRef: null },
    request: "First line\n\nSecond line",
    viewport: { width: 800, height: 600 },
  });
  const parsed = parseReviewFeedback({
    id: FEEDBACK_EVENT,
    pubkey: REVIEWER,
    created_at: 1_700_000_200,
    kind: 45010,
    tags: template.tags,
    content: template.content,
    sig: "f".repeat(128),
  });
  assert.equal(parsed.request, "First line\n\nSecond line");
  assert.equal(parsed.reviewed.buzzRevisionEventId, REVISION_EVENT);
  assert.equal(parsed.target.reviewId, "checkout.summary");
});

test("feedback whose filter tags disagree with its signed content is rejected", () => {
  const revision = revisionFixture();
  const good = feedbackEvent({ revision });
  assert.doesNotThrow(() => parseReviewFeedback(good));

  const retag = (name, value) => ({
    ...good,
    tags: good.tags.map((tag) => (tag[0] === name ? [name, value] : tag)),
  });
  assert.throws(() => parseReviewFeedback(retag("target", "other.block")));
  assert.throws(() =>
    parseReviewFeedback(retag("target_revision", REVISION_TWO_EVENT)),
  );
  assert.throws(() => parseReviewFeedback(retag("payload", "9".repeat(64))));
  assert.throws(() =>
    parseReviewFeedback({
      ...good,
      tags: good.tags.map((tag) =>
        tag[0] === "type" ? ["type", "synaxis.html-review"] : tag,
      ),
    }),
  );
});

test("a feedback listing keeps valid entries and counts the invalid", () => {
  const revision = revisionFixture();
  const good = feedbackEvent({ revision });
  const bad = {
    ...feedbackEvent({ revision, eventId: "5".repeat(64) }),
    content: "nope",
  };
  const listing = parseReviewFeedbackList([good, bad]);
  assert.equal(listing.feedback.length, 1);
  assert.equal(listing.invalid, 1);
});

// Identifiers the relay revalidates as tags are bounded in UTF-8 bytes (128).
// Each multi-byte overflow is still at most 128 UTF-16 units, so a
// `value.length` check would have admitted it.
const IDENTIFIER_CASES = [
  ["ASCII", "a".repeat(128), "a".repeat(129)],
  ["two-byte", "é".repeat(64), "é".repeat(65)],
  ["three-byte", `${"€".repeat(42)}ab`, "€".repeat(43)],
  ["four-byte surrogate pair", "😀".repeat(32), "😀".repeat(33)],
];

test("a review's Synaxis artifact ID is bounded by UTF-8 bytes", () => {
  const withArtifactId = (id) => {
    const event = reviewEvent();
    const content = JSON.parse(event.content);
    content.artifact.id = id;
    return { ...event, content: JSON.stringify(content) };
  };
  for (const [name, fits, tooLong] of IDENTIFIER_CASES) {
    assert.equal(
      parseReviewRevision(withArtifactId(fits)).synaxisArtifact.id,
      fits,
      name,
    );
    assert.throws(
      () => parseReviewRevision(withArtifactId(tooLong)),
      /artifact\.id/,
      name,
    );
  }
});

test("feedback identifiers mirrored into relay tags are bounded by UTF-8 bytes", () => {
  const base = feedbackEvent({ revision: revisionFixture() });
  const withIdentifiers = ({ synaxisArtifactId, reviewId }) => {
    const content = JSON.parse(base.content);
    content.reviewed.synaxis_artifact_id = synaxisArtifactId;
    content.target.review_id = reviewId;
    return {
      ...base,
      content: JSON.stringify(content),
      tags: base.tags.map((tag) => {
        if (tag[0] === "synaxis_artifact") return [tag[0], synaxisArtifactId];
        if (tag[0] === "target") return [tag[0], reviewId];
        return tag;
      }),
    };
  };
  for (const [name, fits, tooLong] of IDENTIFIER_CASES) {
    const parsed = parseReviewFeedback(
      withIdentifiers({ synaxisArtifactId: fits, reviewId: fits }),
    );
    assert.equal(parsed.reviewed.synaxisArtifactId, fits, name);
    assert.equal(parsed.target.reviewId, fits, name);
    assert.throws(
      () =>
        parseReviewFeedback(
          withIdentifiers({ synaxisArtifactId: tooLong, reviewId: "ok" }),
        ),
      /synaxis_artifact_id/,
      name,
    );
    assert.throws(
      () =>
        parseReviewFeedback(
          withIdentifiers({ synaxisArtifactId: "ok", reviewId: tooLong }),
        ),
      /review_id/,
      name,
    );
  }
});

test("an over-long identifier is refused before feedback is signed", () => {
  const revision = revisionFixture();
  const block = {
    id: "checkout.primary-action",
    title: "Primary checkout action",
    sourceRef: null,
  };
  const build = (overrides) =>
    buildFeedbackArtifact({
      feedbackArtifactId: FEEDBACK_ID,
      revision: { ...revision, ...overrides.revision },
      block: { ...block, ...overrides.block },
      request: "Move this.",
      viewport: { width: 100, height: 100 },
    });
  for (const [name, fits, tooLong] of IDENTIFIER_CASES) {
    const accepted = build({
      revision: { synaxisArtifact: { ...revision.synaxisArtifact, id: fits } },
      block: { id: fits },
    });
    assert.equal(
      accepted.tags.find((tag) => tag[0] === "synaxis_artifact")[1],
      fits,
      name,
    );
    assert.throws(
      () =>
        build({
          revision: {
            synaxisArtifact: { ...revision.synaxisArtifact, id: tooLong },
          },
          block: {},
        }),
      /reviewed artifact ID/,
      name,
    );
    assert.throws(
      () => build({ revision: {}, block: { id: tooLong } }),
      /review block ID/,
      name,
    );
  }
});

// ── Block markers ───────────────────────────────────────────────────────────

test("source references must be normalized workspace-relative paths", () => {
  for (const ok of [
    "src/features/checkout/CheckoutActions.tsx",
    "README.md",
    "a/b.c/d-e_f",
  ]) {
    assert.equal(isWorkspaceRelativePath(ok), true, ok);
  }
  for (const bad of [
    "",
    "/etc/passwd",
    "C:\\Windows\\system.ini",
    "c:/x",
    "../secrets.txt",
    "src/../../secrets.txt",
    "src/./a.ts",
    "src//a.ts",
    "src\\a.ts",
    "src/a.ts\n",
    `${"a/".repeat(300)}b`,
  ]) {
    assert.equal(isWorkspaceRelativePath(bad), false, JSON.stringify(bad));
  }
});

test("block markers need a safe id, a nonblank title, and a valid source ref", () => {
  assert.deepEqual(
    validateReviewBlock({
      id: "checkout.primary-action",
      title: " Pay ",
      sourceRef: null,
    }),
    { id: "checkout.primary-action", title: "Pay", sourceRef: null },
  );
  assert.throws(() =>
    validateReviewBlock({ id: null, title: "x", sourceRef: null }),
  );
  assert.throws(() =>
    validateReviewBlock({ id: 'a"b', title: "x", sourceRef: null }),
  );
  assert.throws(() =>
    validateReviewBlock({ id: "ok", title: "   ", sourceRef: null }),
  );
  assert.throws(() =>
    validateReviewBlock({ id: "ok", title: null, sourceRef: null }),
  );
  assert.throws(() =>
    validateReviewBlock({
      id: "ok",
      title: "x",
      sourceRef: "../../etc/passwd",
    }),
  );
});

// ── Joining feedback to the review it claims ────────────────────────────────

test("feedback binds only when every signed invariant matches the loaded review and its block table", () => {
  const revision = revisionFixture();
  const blocks = [
    { id: "checkout.header", title: "Checkout header", sourceRef: null },
    {
      id: "checkout.primary-action",
      title: "Primary checkout action",
      sourceRef: null,
    },
  ];
  const good = parseReviewFeedback(feedbackEvent({ revision }));
  // The trusted block entry is returned; callers render it, not the event text.
  assert.equal(bindFeedbackToReview(good, revision, blocks), blocks[1]);

  const withReviewed = (field, value) => {
    const event = feedbackEvent({ revision });
    const content = JSON.parse(event.content);
    content.reviewed[field] = value;
    const tagFor = {
      synaxis_artifact_id: "synaxis_artifact",
      payload_digest: "payload",
      buzz_revision_event_id: "target_revision",
    }[field];
    return parseReviewFeedback({
      ...event,
      content: JSON.stringify(content),
      tags: event.tags.map((tag) =>
        tag[0] === tagFor ? [tag[0], value] : tag,
      ),
    });
  };
  const rejected = {
    "other synaxis artifact": withReviewed("synaxis_artifact_id", "01FORGED"),
    "other payload digest": withReviewed("payload_digest", "9".repeat(64)),
    "other revision": withReviewed(
      "buzz_revision_event_id",
      REVISION_TWO_EVENT,
    ),
  };
  for (const [name, feedback] of Object.entries(rejected)) {
    assert.throws(
      () => bindFeedbackToReview(feedback, revision, blocks),
      /not bound/,
      name,
    );
  }

  // A different Buzz artifact UUID.
  const otherArtifact = {
    ...good,
    reviewed: { ...good.reviewed, buzzArtifactId: FEEDBACK_ID },
  };
  assert.throws(
    () => bindFeedbackToReview(otherArtifact, revision, blocks),
    /not bound/,
  );

  // Undeclared block, or declared block with fabricated metadata.
  const stranger = parseReviewFeedback(
    feedbackEvent({ revision, blockId: "invented.block", title: "Invented" }),
  );
  assert.throws(
    () => bindFeedbackToReview(stranger, revision, blocks),
    /does not declare/,
  );
  const retitled = parseReviewFeedback(
    feedbackEvent({ revision, title: "Totally different title" }),
  );
  assert.throws(
    () => bindFeedbackToReview(retitled, revision, blocks),
    /does not declare/,
  );
  assert.throws(
    () =>
      bindFeedbackToReview(good, revision, [
        { ...blocks[1], sourceRef: "src/other.tsx" },
      ]),
    /does not declare/,
  );
});
