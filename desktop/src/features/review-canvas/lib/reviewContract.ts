import type { RelayEvent } from "@/shared/api/types";
import { KIND_ARTIFACT, KIND_STREAM_MESSAGE } from "@/shared/constants/kinds";

/** NIP-AR `type` of a factory-generated, hash-bound HTML review revision. */
export const REVIEW_ARTIFACT_TYPE = "synaxis.html-review";
/** NIP-AR `type` of a human feedback revision bound to a reviewed revision. */
export const FEEDBACK_ARTIFACT_TYPE = "synaxis.artifact-feedback";
export const REVIEW_SCHEMA = "synaxis.html-review/v1";
export const FEEDBACK_SCHEMA = "synaxis.artifact-feedback/v1";
/** Mirrors `MAX_REVIEW_DOCUMENT_BYTES` in the native artifact module. */
export const MAX_REVIEW_DOCUMENT_BYTES = 4 * 1024 * 1024;
export const MAX_FEEDBACK_REQUEST_BYTES = 8 * 1024;
/** Mirrors Synaxis' maximum cumulative revision dispositions. */
export const MAX_DISPOSITIONS = 64;
const MAX_EVENT_CONTENT_BYTES = 64 * 1024;
/** The relay bounds mirrored identifier tags by UTF-8 bytes, not UTF-16 units. */
export const MAX_IDENTIFIER_BYTES = 128;
const MAX_IDENTIFIER_CHARS = 128;
const MAX_TITLE_CHARS = 200;

export type ReviewDispositionStatus = "addressed" | "unresolved";

export type ReviewDisposition = {
  feedbackRevisionEventId: string;
  status: ReviewDispositionStatus;
};

/** A declared review block: the only thing a reviewer may comment on. */
export type ReviewBlock = {
  id: string;
  title: string;
  sourceRef: string | null;
};

/** Raised for any artifact, notification, or document that breaks the contract. */
export class ReviewContractError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ReviewContractError";
  }
}

const HEX64 = /^[0-9a-f]{64}$/;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const NIL_UUID = "00000000-0000-0000-0000-000000000000";

/** 64 lowercase hex characters: Nostr event IDs, pubkeys, and SHA-256 digests. */
export function isHex64(value: unknown): value is string {
  return typeof value === "string" && HEX64.test(value);
}

/** Canonical lowercase, hyphenated, non-nil UUID (the NIP-AR `d`/`h` shape). */
export function isCanonicalUuid(value: unknown): value is string {
  return typeof value === "string" && UUID.test(value) && value !== NIL_UUID;
}

function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).length;
}

function hasControlCharacter(value: string): boolean {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point.
  return /[\u0000-\u001f\u007f]/.test(value);
}

/** The only tag named `name`, or `undefined` when absent. Duplicates throw. */
function singleTag(tags: string[][], name: string): string[] | undefined {
  const matches = tags.filter((tag) => tag[0] === name);
  if (matches.length > 1) {
    throw new ReviewContractError(`Duplicate "${name}" tag.`);
  }
  return matches[0];
}

function requiredTagValue(tags: string[][], name: string): string {
  const tag = singleTag(tags, name);
  if (tag?.length !== 2 || tag[1].length === 0) {
    throw new ReviewContractError(`Missing or malformed "${name}" tag.`);
  }
  return tag[1];
}

function asRecord(value: unknown, label: string): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ReviewContractError(`${label} must be an object.`);
  }
  return value as Record<string, unknown>;
}

function boundedString(
  value: unknown,
  label: string,
  maxChars = MAX_IDENTIFIER_CHARS,
): string {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    value.length > maxChars ||
    hasControlCharacter(value)
  ) {
    throw new ReviewContractError(`${label} is missing or malformed.`);
  }
  return value;
}

/**
 * An identifier the relay revalidates as a tag value (`synaxis_artifact`,
 * `target`): nonblank, free of control characters, and at most
 * `MAX_IDENTIFIER_BYTES` UTF-8 bytes. A UTF-16 length check would admit
 * multi-byte text that the signed feedback would then fail on at the relay.
 */
function boundedIdentifier(value: unknown, label: string): string {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    utf8ByteLength(value) > MAX_IDENTIFIER_BYTES ||
    hasControlCharacter(value)
  ) {
    throw new ReviewContractError(`${label} is missing or malformed.`);
  }
  return value;
}

/**
 * Multi-line human text: nonblank and bounded in UTF-8 bytes, rejecting
 * control characters other than tab, newline, and carriage return.
 */
function boundedText(value: unknown, label: string, maxBytes: number): string {
  if (
    typeof value !== "string" ||
    value.trim().length === 0 ||
    utf8ByteLength(value) > maxBytes ||
    // biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point.
    /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value)
  ) {
    throw new ReviewContractError(`${label} is missing or malformed.`);
  }
  return value;
}

function hexField(value: unknown, label: string): string {
  if (!isHex64(value)) {
    throw new ReviewContractError(
      `${label} must be 64 lowercase hex characters.`,
    );
  }
  return value;
}

function positiveInteger(value: unknown, label: string, max: number): number {
  if (
    !Number.isSafeInteger(value) ||
    (value as number) < 1 ||
    (value as number) > max
  ) {
    throw new ReviewContractError(
      `${label} must be an integer from 1 to ${max}.`,
    );
  }
  return value as number;
}

function parseJsonContent(content: string, label: string) {
  if (utf8ByteLength(content) > MAX_EVENT_CONTENT_BYTES) {
    throw new ReviewContractError(`${label} content is too large.`);
  }
  try {
    return asRecord(JSON.parse(content), `${label} content`);
  } catch (error) {
    if (error instanceof ReviewContractError) throw error;
    throw new ReviewContractError(`${label} content is not valid JSON.`);
  }
}

// ── Notification (kind 9) ────────────────────────────────────────────────────

export type ReviewNotification = {
  /** The kind-9 notification message, which anchors the original thread. */
  messageId: string;
  /** Stable NIP-AR artifact UUID (`d`). */
  artifactId: string;
  /** Exact review revision the notification announces. */
  revisionEventId: string;
  /** Pubkey of the executive agent feedback must wake. */
  executiveAgentPubkey: string;
  /** Reviewed payload digest the notification promises. */
  payloadDigest: string;
  /** Signer of the notification; the revision must come from the same signer. */
  authorPubkey: string | null;
  /** Thread root a reply must attach to: the message's root, else the message. */
  threadRootId: string;
};

type NotificationSource = {
  id: string;
  kind?: number;
  tags?: string[][];
  signerPubkey?: string;
  pubkey?: string;
  parentId?: string | null;
  rootId?: string | null;
};

/**
 * Recognize a review notification. Only a kind-9 message carrying exactly one
 * well-formed `artifact`, `artifact_type`, `agent`, and `x` tag qualifies;
 * anything else is an ordinary message and renders as one. Recognition does
 * not trust the notification: the canvas re-verifies the signed revision.
 */
export function parseReviewNotification(
  message: NotificationSource,
): ReviewNotification | null {
  if (
    message.kind !== KIND_STREAM_MESSAGE ||
    !message.tags ||
    !isHex64(message.id)
  ) {
    return null;
  }
  try {
    const artifact = singleTag(message.tags, "artifact");
    if (
      artifact?.length !== 3 ||
      !isCanonicalUuid(artifact[1]) ||
      !isHex64(artifact[2])
    ) {
      return null;
    }
    const artifactType = singleTag(message.tags, "artifact_type");
    if (
      artifactType?.length !== 2 ||
      artifactType[1] !== REVIEW_ARTIFACT_TYPE
    ) {
      return null;
    }
    const agent = singleTag(message.tags, "agent");
    const digest = singleTag(message.tags, "x");
    if (
      agent?.length !== 2 ||
      !isHex64(agent[1]) ||
      digest?.length !== 2 ||
      !isHex64(digest[1])
    ) {
      return null;
    }
    return {
      messageId: message.id,
      artifactId: artifact[1],
      revisionEventId: artifact[2],
      executiveAgentPubkey: agent[1],
      payloadDigest: digest[1],
      authorPubkey: message.signerPubkey ?? message.pubkey ?? null,
      threadRootId: message.rootId ?? message.parentId ?? message.id,
    };
  } catch (error) {
    if (error instanceof ReviewContractError) return null;
    throw error;
  }
}

// ── Review revision (kind 45010, synaxis.html-review) ───────────────────────

export type ReviewRevision = {
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  channelId: string;
  artifactId: string;
  title: string;
  rootEventId: string;
  prevEventId: string | null;
  workItemId: string;
  runId: string;
  /** 1-based revision number the factory assigned. */
  revision: number;
  attribution: {
    requestedBy: string;
    submittedBy: string;
    originEventId: string;
  };
  synaxisArtifact: {
    id: string;
    kind: string;
    schemaVersion: number;
    payloadDigest: string;
  };
  document: { url: string; sha256: string; size: number };
  dispositions: ReviewDisposition[];
};

function parseImeta(tags: string[][]): Map<string, string> {
  const imeta = singleTag(tags, "imeta");
  if (!imeta) {
    throw new ReviewContractError(
      "The review has no document attachment (imeta).",
    );
  }
  const fields = new Map<string, string>();
  for (const part of imeta.slice(1)) {
    const space = part.indexOf(" ");
    if (space <= 0) continue;
    const key = part.slice(0, space);
    if (fields.has(key)) {
      throw new ReviewContractError(`Duplicate imeta "${key}" field.`);
    }
    fields.set(key, part.slice(space + 1));
  }
  return fields;
}

function parseDispositions(value: unknown): ReviewDisposition[] {
  if (!Array.isArray(value) || value.length > MAX_DISPOSITIONS) {
    throw new ReviewContractError(
      "feedback_dispositions is missing or too long.",
    );
  }
  const seen = new Set<string>();
  return value.map((entry) => {
    const record = asRecord(entry, "A feedback disposition");
    const feedbackRevisionEventId = hexField(
      record.feedback_revision_event_id,
      "feedback_revision_event_id",
    );
    if (record.status !== "addressed" && record.status !== "unresolved") {
      throw new ReviewContractError(
        'A feedback disposition status must be "addressed" or "unresolved".',
      );
    }
    if (seen.has(feedbackRevisionEventId)) {
      throw new ReviewContractError("Duplicate feedback disposition.");
    }
    seen.add(feedbackRevisionEventId);
    return { feedbackRevisionEventId, status: record.status };
  });
}

/**
 * Interpret a natively verified event as a `synaxis.html-review/v1` revision.
 * The digest agreement rule lives here: the content's blob hash, the `imeta`
 * hash, and the Synaxis artifact payload digest must be the same value.
 */
export function parseReviewRevision(event: RelayEvent): ReviewRevision {
  if (
    event.kind !== KIND_ARTIFACT ||
    !isHex64(event.id) ||
    !isHex64(event.pubkey)
  ) {
    throw new ReviewContractError(
      "The event is not a NIP-AR artifact revision.",
    );
  }
  const tags = event.tags;
  if (requiredTagValue(tags, "ar") !== "1") {
    throw new ReviewContractError("Unsupported artifact envelope version.");
  }
  if (requiredTagValue(tags, "type") !== REVIEW_ARTIFACT_TYPE) {
    throw new ReviewContractError("The artifact is not an HTML review.");
  }
  const artifactId = requiredTagValue(tags, "d");
  const channelId = requiredTagValue(tags, "h");
  const op = requiredTagValue(tags, "op");
  if (!isCanonicalUuid(artifactId) || !isCanonicalUuid(channelId)) {
    throw new ReviewContractError("The artifact identity is malformed.");
  }
  if (op === "delete") {
    throw new ReviewContractError("This review artifact was deleted.");
  }
  const title = boundedString(
    requiredTagValue(tags, "title"),
    "Review title",
    512,
  );
  const root = singleTag(tags, "root");
  const prev = singleTag(tags, "prev");
  if (root?.length !== 2 || !isHex64(root[1]) || (prev && !isHex64(prev[1]))) {
    throw new ReviewContractError("The artifact lineage tags are malformed.");
  }

  const content = parseJsonContent(event.content, "Review");
  if (content.schema !== REVIEW_SCHEMA) {
    throw new ReviewContractError("Unsupported review schema.");
  }
  const attribution = asRecord(content.attribution, "attribution");
  const artifact = asRecord(content.artifact, "artifact");
  const presentation = asRecord(content.presentation, "presentation");
  if (presentation.mime_type !== "text/html") {
    throw new ReviewContractError("The review presentation is not HTML.");
  }
  const payloadDigest = hexField(
    artifact.payload_digest,
    "artifact.payload_digest",
  );
  const sha256 = hexField(presentation.blob_sha256, "presentation.blob_sha256");
  const size = positiveInteger(
    presentation.byte_size,
    "presentation.byte_size",
    MAX_REVIEW_DOCUMENT_BYTES,
  );

  const imeta = parseImeta(tags);
  const imetaUrl = imeta.get("url");
  if (!imetaUrl || !/^https?:\/\//.test(imetaUrl) || imetaUrl.length > 2048) {
    throw new ReviewContractError("The review attachment URL is malformed.");
  }
  if (imeta.get("m") !== undefined && imeta.get("m") !== "text/html") {
    throw new ReviewContractError("The review attachment is not HTML.");
  }
  if (imeta.get("size") !== undefined && Number(imeta.get("size")) !== size) {
    throw new ReviewContractError(
      "The attachment size disagrees with the review.",
    );
  }
  if (new Set([payloadDigest, sha256, imeta.get("x")]).size !== 1) {
    throw new ReviewContractError(
      "The review's payload digest, content hash, and attachment hash disagree.",
    );
  }

  return {
    eventId: event.id,
    authorPubkey: event.pubkey,
    createdAt: event.created_at,
    channelId,
    artifactId,
    title,
    rootEventId: root[1],
    prevEventId: prev ? prev[1] : null,
    workItemId: boundedString(content.work_item_id, "work_item_id"),
    runId: boundedString(content.run_id, "run_id"),
    revision: positiveInteger(content.revision, "revision", 1_000_000),
    attribution: {
      requestedBy: hexField(
        attribution.requested_by,
        "attribution.requested_by",
      ),
      submittedBy: hexField(
        attribution.submitted_by,
        "attribution.submitted_by",
      ),
      originEventId: hexField(
        attribution.origin_event_id,
        "attribution.origin_event_id",
      ),
    },
    synaxisArtifact: {
      id: boundedIdentifier(artifact.id, "artifact.id"),
      kind: boundedString(artifact.kind, "artifact.kind"),
      schemaVersion: positiveInteger(
        artifact.schema_version,
        "artifact.schema_version",
        1_000,
      ),
      payloadDigest,
    },
    document: { url: imetaUrl, sha256, size },
    dispositions: parseDispositions(content.feedback_dispositions),
  };
}

// ── Feedback revision (kind 45010, synaxis.artifact-feedback) ───────────────

export type ReviewFeedback = {
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  artifactId: string;
  reviewed: {
    buzzArtifactId: string;
    buzzRevisionEventId: string;
    synaxisArtifactId: string;
    payloadDigest: string;
  };
  target: { reviewId: string; title: string; sourceRef: string | null };
  request: string;
};

/** Normalized workspace-relative path: no absolute roots, traversal, or dots. */
export function isWorkspaceRelativePath(value: string): boolean {
  if (
    value.length === 0 ||
    value.length > 512 ||
    hasControlCharacter(value) ||
    value.includes("\\") ||
    value.startsWith("/") ||
    /^[A-Za-z]:/.test(value)
  ) {
    return false;
  }
  return value
    .split("/")
    .every(
      (segment) => segment.length > 0 && segment !== "." && segment !== "..",
    );
}

/** Review block IDs: stable, shell- and path-inert identifiers. */
export function isReviewBlockId(value: string): boolean {
  return /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(value);
}

export function validateReviewBlock(block: {
  id: string | null;
  title: string | null;
  sourceRef: string | null;
}): ReviewBlock {
  if (!block.id || !isReviewBlockId(block.id)) {
    throw new ReviewContractError(
      `Review block ID ${JSON.stringify(block.id ?? "")} is not a valid identifier.`,
    );
  }
  const title = block.title?.trim() ?? "";
  if (
    title.length === 0 ||
    title.length > MAX_TITLE_CHARS ||
    hasControlCharacter(title)
  ) {
    throw new ReviewContractError(
      `Review block "${block.id}" needs a nonblank title.`,
    );
  }
  if (block.sourceRef !== null && !isWorkspaceRelativePath(block.sourceRef)) {
    throw new ReviewContractError(
      `Review block "${block.id}" has a source reference that is not a workspace-relative path.`,
    );
  }
  return { id: block.id, title, sourceRef: block.sourceRef };
}

/**
 * Interpret a natively verified event as a human feedback revision. The
 * filterable tags must restate the signed content exactly, so a mismatched
 * tag can never redirect feedback to a different block or revision.
 */
export function parseReviewFeedback(event: RelayEvent): ReviewFeedback {
  if (
    event.kind !== KIND_ARTIFACT ||
    !isHex64(event.id) ||
    !isHex64(event.pubkey)
  ) {
    throw new ReviewContractError(
      "The event is not a NIP-AR artifact revision.",
    );
  }
  const tags = event.tags;
  if (requiredTagValue(tags, "type") !== FEEDBACK_ARTIFACT_TYPE) {
    throw new ReviewContractError("The artifact is not review feedback.");
  }
  const artifactId = requiredTagValue(tags, "d");
  if (!isCanonicalUuid(artifactId)) {
    throw new ReviewContractError("The feedback identity is malformed.");
  }
  const content = parseJsonContent(event.content, "Feedback");
  if (content.schema !== FEEDBACK_SCHEMA) {
    throw new ReviewContractError("Unsupported feedback schema.");
  }
  const reviewed = asRecord(content.reviewed, "reviewed");
  const target = asRecord(content.target, "target");
  if (!isCanonicalUuid(reviewed.buzz_artifact_id)) {
    throw new ReviewContractError("reviewed.buzz_artifact_id is malformed.");
  }
  const parsed: ReviewFeedback = {
    eventId: event.id,
    authorPubkey: event.pubkey,
    createdAt: event.created_at,
    artifactId,
    reviewed: {
      buzzArtifactId: reviewed.buzz_artifact_id,
      buzzRevisionEventId: hexField(
        reviewed.buzz_revision_event_id,
        "reviewed.buzz_revision_event_id",
      ),
      synaxisArtifactId: boundedIdentifier(
        reviewed.synaxis_artifact_id,
        "reviewed.synaxis_artifact_id",
      ),
      payloadDigest: hexField(
        reviewed.payload_digest,
        "reviewed.payload_digest",
      ),
    },
    target: {
      reviewId: boundedIdentifier(target.review_id, "target.review_id"),
      title: boundedString(target.title, "target.title", MAX_TITLE_CHARS),
      sourceRef:
        target.source_ref === undefined
          ? null
          : boundedString(target.source_ref, "target.source_ref", 512),
    },
    request: boundedText(
      content.request,
      "request",
      MAX_FEEDBACK_REQUEST_BYTES,
    ),
  };
  if (
    requiredTagValue(tags, "target") !== parsed.target.reviewId ||
    requiredTagValue(tags, "target_revision") !==
      parsed.reviewed.buzzRevisionEventId ||
    requiredTagValue(tags, "synaxis_artifact") !==
      parsed.reviewed.synaxisArtifactId ||
    requiredTagValue(tags, "payload") !== parsed.reviewed.payloadDigest
  ) {
    throw new ReviewContractError(
      "The feedback tags disagree with its signed content.",
    );
  }
  return parsed;
}

/** Parse a listing, keeping valid feedback and counting what failed the contract. */
export function parseReviewFeedbackList(events: RelayEvent[]): {
  feedback: ReviewFeedback[];
  invalid: number;
} {
  const feedback: ReviewFeedback[] = [];
  let invalid = 0;
  for (const event of events) {
    try {
      feedback.push(parseReviewFeedback(event));
    } catch (error) {
      if (!(error instanceof ReviewContractError)) throw error;
      invalid += 1;
    }
  }
  return { feedback, invalid };
}

// ── Feedback construction ────────────────────────────────────────────────────

export type FeedbackViewport = { width: number; height: number };

export type FeedbackArtifactInput = {
  feedbackArtifactId: string;
  revision: ReviewRevision;
  block: ReviewBlock;
  request: string;
  viewport: FeedbackViewport;
};

export type FeedbackArtifactTemplate = {
  kind: typeof KIND_ARTIFACT;
  content: string;
  tags: string[][];
};

/** Validate and normalize a reviewer's comment; throws a user-facing error. */
export function normalizeFeedbackRequest(request: string): string {
  const trimmed = request.trim();
  if (trimmed.length === 0) {
    throw new ReviewContractError("Write a comment before sending feedback.");
  }
  if (utf8ByteLength(trimmed) > MAX_FEEDBACK_REQUEST_BYTES) {
    throw new ReviewContractError(
      `Comments are limited to ${MAX_FEEDBACK_REQUEST_BYTES / 1024} KiB.`,
    );
  }
  return trimmed;
}

/** Return the longest whole-code-point prefix within a UTF-8 byte limit. */
function truncateUtf8(value: string, maxBytes: number): string {
  const characters: string[] = [];
  let bytes = 0;
  for (const character of value) {
    const characterBytes = utf8ByteLength(character);
    if (bytes + characterBytes > maxBytes) break;
    characters.push(character);
    bytes += characterBytes;
  }
  return characters.join("");
}

function clampViewport(viewport: FeedbackViewport): FeedbackViewport {
  const clamp = (value: number) =>
    Math.min(
      10_000,
      Math.max(1, Math.round(Number.isFinite(value) ? value : 1)),
    );
  return { width: clamp(viewport.width), height: clamp(viewport.height) };
}

/**
 * Build the unsigned `synaxis.artifact-feedback` NIP-AR revision bound to the
 * exact reviewed revision, payload digest, and declared block. No document
 * bytes are copied: the feedback names what it is about, never contains it.
 */
export function buildFeedbackArtifact(
  input: FeedbackArtifactInput,
): FeedbackArtifactTemplate {
  const { revision, block } = input;
  if (!isCanonicalUuid(input.feedbackArtifactId)) {
    throw new ReviewContractError("The feedback identity is malformed.");
  }
  // Both identifiers are mirrored into relay-validated tags: refuse an
  // over-long one before it is signed rather than after.
  boundedIdentifier(revision.synaxisArtifact.id, "The reviewed artifact ID");
  boundedIdentifier(block.id, "The review block ID");
  const request = normalizeFeedbackRequest(input.request);
  const content = JSON.stringify({
    schema: FEEDBACK_SCHEMA,
    reviewed: {
      buzz_artifact_id: revision.artifactId,
      buzz_revision_event_id: revision.eventId,
      synaxis_artifact_id: revision.synaxisArtifact.id,
      payload_digest: revision.synaxisArtifact.payloadDigest,
    },
    target: {
      review_id: block.id,
      title: block.title,
      ...(block.sourceRef ? { source_ref: block.sourceRef } : {}),
    },
    context: {
      viewport: clampViewport(input.viewport),
      // Only explicitly instrumented controls may contribute state; the pilot
      // documents have none, so the object is always empty.
      interaction_state: {},
    },
    request,
  });
  const tags: string[][] = [
    ["ar", "1"],
    ["d", input.feedbackArtifactId],
    ["h", revision.channelId],
    ["type", FEEDBACK_ARTIFACT_TYPE],
    ["title", truncateUtf8(`Feedback: ${block.title}`, 512)],
    ["op", "create"],
    ["root", revision.rootEventId],
    ["target", block.id],
    ["target_revision", revision.eventId],
    ["synaxis_artifact", revision.synaxisArtifact.id],
    ["payload", revision.synaxisArtifact.payloadDigest],
  ];
  return { kind: KIND_ARTIFACT, content, tags };
}

// ── Joining feedback to the review it claims ─────────────────────────────────

/**
 * Check a parsed feedback entry against the review revision (and its sanitized
 * block table) it claims to be about, and return the *trusted* block entry.
 *
 * Self-consistency (tags restating content) proves nothing about the loaded
 * review, so every signed invariant is compared: the Buzz artifact UUID, the
 * exact revision event, the Synaxis artifact ID, the payload digest, and a
 * declared block whose ID, title, and source reference all match exactly.
 * Callers render the returned block, never the event-supplied metadata.
 */
export function bindFeedbackToReview(
  feedback: ReviewFeedback,
  revision: ReviewRevision,
  blocks: ReviewBlock[],
): ReviewBlock {
  const { reviewed, target } = feedback;
  if (
    reviewed.buzzArtifactId !== revision.artifactId ||
    reviewed.buzzRevisionEventId !== revision.eventId ||
    reviewed.synaxisArtifactId !== revision.synaxisArtifact.id ||
    reviewed.payloadDigest !== revision.synaxisArtifact.payloadDigest
  ) {
    throw new ReviewContractError(
      "The feedback is not bound to this review revision.",
    );
  }
  const block = blocks.find((candidate) => candidate.id === target.reviewId);
  if (
    !block ||
    block.title !== target.title ||
    block.sourceRef !== target.sourceRef
  ) {
    throw new ReviewContractError(
      "The feedback names a block this review does not declare.",
    );
  }
  return block;
}
