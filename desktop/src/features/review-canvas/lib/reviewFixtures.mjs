// Shared fixtures for the Review Canvas unit tests. Not a test file itself.
import { createHash } from "node:crypto";

export const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
export const ARTIFACT_ID = "04737c81-e5e8-4412-bb47-f446813cfeba";
export const FEEDBACK_ID = "24737c81-e5e8-4412-bb47-f446813cfeba";
export const ADAPTER = "a".repeat(64);
export const REVIEWER = "b".repeat(64);
export const AGENT = "c".repeat(64);
export const ORIGIN_EVENT = "d".repeat(64);
export const NOTIFICATION_ID = "e".repeat(64);
export const REVISION_EVENT = "1".repeat(64);
export const REVISION_TWO_EVENT = "2".repeat(64);
export const FEEDBACK_EVENT = "3".repeat(64);

export const THREE_BLOCK_HTML = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Checkout</title>
<style>.card{padding:8px;background:#fff}</style>
</head>
<body class="page">
<header data-synaxis-review-id="checkout.header" data-synaxis-review-title="Checkout header"><h1>Checkout</h1></header>
<section class="card" data-synaxis-review-id="checkout.primary-action" data-synaxis-review-title="Primary checkout action" data-synaxis-source-ref="src/features/checkout/CheckoutActions.tsx"><button type="button">Pay now</button></section>
<section data-synaxis-review-id="checkout.summary" data-synaxis-review-title="Order summary"><p>Two items</p></section>
</body>
</html>`;

export function sha256(text) {
  return createHash("sha256").update(text).digest("hex");
}

export function byteLength(text) {
  return new TextEncoder().encode(text).length;
}

/**
 * An in-memory `Storage` stand-in for the durable outbox. `data` is the raw
 * backing map, so a test can read exactly what would be on disk, and `broken`
 * makes every write throw like a full or disabled disk.
 */
export function memoryStorage(initial = {}) {
  const data = new Map(Object.entries(initial));
  const storage = {
    data,
    broken: false,
    getItem: (key) => data.get(key) ?? null,
    setItem: (key, value) => {
      if (storage.broken) throw new Error("QuotaExceededError");
      data.set(key, String(value));
    },
    removeItem: (key) => {
      data.delete(key);
    },
  };
  return storage;
}

/** A well-formed `synaxis.html-review/v1` NIP-AR event for `html`. */
export function reviewEvent({
  html = THREE_BLOCK_HTML,
  eventId = REVISION_EVENT,
  revision = 1,
  dispositions = [],
  overrides = {},
  tagOverrides = {},
} = {}) {
  const digest = sha256(html);
  const content = {
    schema: "synaxis.html-review/v1",
    work_item_id: "01WORKITEM",
    run_id: "01RUN",
    revision,
    attribution: {
      requested_by: REVIEWER,
      submitted_by: ADAPTER,
      origin_event_id: ORIGIN_EVENT,
    },
    artifact: {
      id: "01SYNAXISARTIFACT",
      kind: "feature-delivery/v3:HtmlReview",
      schema_version: 1,
      payload_digest: digest,
    },
    presentation: {
      mime_type: "text/html",
      blob_sha256: digest,
      byte_size: byteLength(html),
    },
    feedback_dispositions: dispositions,
    ...overrides,
  };
  const tags = [
    ["ar", "1"],
    ["d", ARTIFACT_ID],
    ["h", CHANNEL_ID],
    ["type", "synaxis.html-review"],
    ["title", "Checkout review"],
    ["op", revision === 1 ? "create" : "update"],
    ["root", ORIGIN_EVENT],
    ...(revision === 1 ? [] : [["prev", REVISION_EVENT]]),
    [
      "imeta",
      `url https://relay.example/media/${digest}.html`,
      "m text/html",
      `x ${digest}`,
      `size ${byteLength(html)}`,
    ],
  ].map((tag) => tagOverrides[tag[0]] ?? tag);
  return {
    id: eventId,
    pubkey: ADAPTER,
    created_at: 1_700_000_000 + revision,
    kind: 45010,
    tags,
    content: JSON.stringify(content),
    sig: "f".repeat(128),
  };
}

/** A kind-9 message carrying the shared review notification tags. */
export function notificationMessage(overrides = {}) {
  return {
    id: NOTIFICATION_ID,
    kind: 9,
    signerPubkey: ADAPTER,
    parentId: ORIGIN_EVENT,
    rootId: ORIGIN_EVENT,
    tags: [
      ["h", CHANNEL_ID],
      ["e", ORIGIN_EVENT, "", "reply"],
      ["p", REVIEWER],
      ["artifact", ARTIFACT_ID, REVISION_EVENT],
      ["artifact_type", "synaxis.html-review"],
      ["agent", AGENT],
      ["x", sha256(THREE_BLOCK_HTML)],
    ],
    ...overrides,
  };
}

/** A signed-looking feedback revision event for `revision`. */
export function feedbackEvent({
  revision,
  blockId = "checkout.primary-action",
  title = "Primary checkout action",
  request = "Move this above the order summary.",
  eventId = FEEDBACK_EVENT,
} = {}) {
  const reviewed = {
    buzz_artifact_id: ARTIFACT_ID,
    buzz_revision_event_id: revision.eventId ?? revision,
    synaxis_artifact_id: "01SYNAXISARTIFACT",
    payload_digest: sha256(THREE_BLOCK_HTML),
  };
  return {
    id: eventId,
    pubkey: REVIEWER,
    created_at: 1_700_000_100,
    kind: 45010,
    tags: [
      ["ar", "1"],
      ["d", FEEDBACK_ID],
      ["h", CHANNEL_ID],
      ["type", "synaxis.artifact-feedback"],
      ["title", `Feedback: ${title}`],
      ["op", "create"],
      ["target", blockId],
      ["target_revision", reviewed.buzz_revision_event_id],
      ["synaxis_artifact", reviewed.synaxis_artifact_id],
      ["payload", reviewed.payload_digest],
    ],
    content: JSON.stringify({
      schema: "synaxis.artifact-feedback/v1",
      reviewed,
      target: { review_id: blockId, title },
      context: {
        viewport: { width: 1440, height: 900 },
        interaction_state: {},
      },
      request,
    }),
    sig: "f".repeat(128),
  };
}
