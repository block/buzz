import { z } from "zod";
import type { RelayEvent } from "@/shared/api/types";
import {
  buildFeedbackArtifact,
  isCanonicalUuid,
  isHex64,
  normalizeFeedbackRequest,
  validateReviewBlock,
} from "./reviewContract";
import type {
  ReviewFeedbackContext,
  ReviewWakeInput,
} from "./reviewFeedbackSubmission";

/**
 * Durable, bounded, device-local outbox of signed review feedback that has not
 * finished: published and woken.
 *
 * What it holds is exactly what the relay already stores or is about to: the
 * signed `synaxis.artifact-feedback` event, the comment, the captured relay
 * URL and signer *public* key, and the frozen inputs of the agent wake. No
 * private key, token, or credential is ever written. It lives in the webview's
 * `localStorage`, so it survives quitting and restarting Buzz, and signing out
 * clears it with the rest of the origin's storage.
 *
 * Bounds: at most {@link OUTBOX_MAX_ENTRIES} unfinished comments per signer,
 * each at most {@link OUTBOX_MAX_RECORD_CHARS} characters (an 8 KiB comment
 * appears twice, once as the comment and once inside the signed event), so the
 * worst case is about 2 MiB of UTF-16 and never grows with use. Entries leave
 * only when feedback and wake both finished, when the reviewer discards a
 * comment the relay provably never stored, or, for such never-stored entries
 * only, after {@link OUTBOX_ABANDONABLE_TTL_SECONDS} of inactivity. An entry
 * the relay may hold is never expired: it is the only record that its wake is
 * owed.
 */

export const OUTBOX_VERSION = 1;
export const OUTBOX_MAX_ENTRIES = 16;
export const OUTBOX_MAX_RECORD_CHARS = 64 * 1024;
export const OUTBOX_ABANDONABLE_TTL_SECONDS = 30 * 24 * 60 * 60;
export const OUTBOX_STORAGE_PREFIX = "buzz-review-outbox.v1:";

/**
 * What is known about the relay holding the signed feedback event.
 *
 * - `never_attempted`: signed and saved, but no publish has started, so the
 *   relay cannot hold it. The only state, besides `rejected`, that may be
 *   discarded.
 * - `ambiguous`: a publish started and ended without an answer from the relay
 *   (timeout, dropped connection, interrupted process). The relay may hold the
 *   event: it must never be discarded or replaced, only reconciled.
 * - `rejected`: the relay answered the exact event with an explicit
 *   "not stored" refusal. Only this, confirmed again by a relay read, proves
 *   the event can be abandoned.
 * - `accepted`: the relay holds the event; only the agent wake can remain.
 */
export type FeedbackPublishState =
  | "never_attempted"
  | "ambiguous"
  | "rejected"
  | "accepted";

/** Everything that fixes which comment on which review a submission is for. */
export type ReviewSubmissionIdentity = {
  /** Relay and signer captured when the reviewer pressed send. */
  relayUrl: string;
  signerPubkey: string;
  channelId: string;
  /** NIP-AR UUID of the reviewed artifact. */
  artifactId: string;
  /** The exact reviewed revision: a revision-1 draft is never revision 2's. */
  revisionEventId: string;
  blockId: string;
};

/** The identity a captured submission context fixes; it never changes. */
export function identityOf(
  context: ReviewFeedbackContext,
): ReviewSubmissionIdentity {
  return {
    relayUrl: context.expectedRelayUrl,
    signerPubkey: context.expectedSignerPubkey,
    channelId: context.revision.channelId,
    artifactId: context.revision.artifactId,
    revisionEventId: context.revision.eventId,
    blockId: context.block.id,
  };
}

export function identityKey(identity: ReviewSubmissionIdentity): string {
  return JSON.stringify([
    identity.relayUrl,
    identity.signerPubkey.toLowerCase(),
    identity.channelId,
    identity.artifactId,
    identity.revisionEventId,
    identity.blockId,
  ]);
}

/** The persisted state of one submission, without its bookkeeping times. */
export type OutboxRecord = {
  context: ReviewFeedbackContext;
  feedbackArtifactId: string;
  signed: RelayEvent;
  publish: FeedbackPublishState;
  /** Frozen wake inputs once the first wake attempt began, else null. */
  wake: ReviewWakeInput | null;
};

/** A stored record plus when it was first saved and last changed (seconds). */
export type OutboxEntry = OutboxRecord & {
  createdAt: number;
  updatedAt: number;
};

export type ReviewOutboxScope = Pick<
  ReviewSubmissionIdentity,
  "relayUrl" | "signerPubkey" | "channelId" | "artifactId"
>;

export type ReviewOutboxFailure = "full" | "too-large" | "unwritable" | "taken";

/** A signed comment could not be made durable; nothing was sent because of it. */
export class ReviewOutboxError extends Error {
  readonly code: ReviewOutboxFailure;

  constructor(
    code: ReviewOutboxFailure,
    message: string,
    options?: ErrorOptions,
  ) {
    super(message, options);
    this.name = "ReviewOutboxError";
    this.code = code;
  }
}

export type ReviewOutbox = {
  /**
   * Throws `ReviewOutboxError` when a new comment for `identity` has no room,
   * or would replace a different comment the relay may already hold.
   */
  assertRoom: (
    identity: ReviewSubmissionIdentity,
    feedbackArtifactId: string,
  ) => void;
  load: (identity: ReviewSubmissionIdentity) => OutboxEntry | undefined;
  /** Durably write; throws `ReviewOutboxError` unless the write is durable. */
  save: (record: OutboxRecord) => void;
  remove: (identity: ReviewSubmissionIdentity) => void;
  /** Unfinished comments on one reviewed artifact, across its revisions. */
  list: (scope: ReviewOutboxScope) => OutboxEntry[];
};

/** The slice of `Storage` the outbox needs; `setItem` must throw on failure. */
export type OutboxStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

// ── Reading records back ────────────────────────────────────────────────────

const hex64Schema = z.custom<string>(isHex64);
const uuidSchema = z.custom<string>(isCanonicalUuid);

const contextSchema = z.object({
  revision: z.looseObject({
    eventId: hex64Schema,
    authorPubkey: hex64Schema,
    rootEventId: hex64Schema,
    channelId: uuidSchema,
    artifactId: uuidSchema,
    title: z.string(),
    revision: z.number().int().min(1),
    synaxisArtifact: z.looseObject({
      id: z.string(),
      payloadDigest: hex64Schema,
    }),
  }),
  block: z.object({
    id: z.string(),
    title: z.string(),
    sourceRef: z.string().nullable(),
  }),
  request: z.string(),
  viewport: z.object({ width: z.number(), height: z.number() }),
  executiveAgentPubkey: hex64Schema,
  parentEventId: hex64Schema,
  rootEventId: hex64Schema,
  expectedRelayUrl: z.string().min(1),
  expectedSignerPubkey: hex64Schema,
});

const wakeSchema = z.object({
  channelId: z.string(),
  content: z.string(),
  parentEventId: z.string(),
  rootEventId: z.string(),
  executiveAgentPubkey: z.string(),
  feedbackArtifactId: z.string(),
  feedbackRevisionEventId: z.string(),
  reviewedArtifactId: z.string(),
  reviewedRevisionEventId: z.string(),
  createdAt: z.number().int().positive(),
  expectedRelayUrl: z.string(),
  expectedSignerPubkey: z.string(),
});

const entrySchema = z.object({
  context: contextSchema,
  feedbackArtifactId: uuidSchema,
  signed: z.object({
    id: hex64Schema,
    pubkey: hex64Schema,
    created_at: z.number(),
    kind: z.number(),
    content: z.string(),
    sig: z.string(),
    tags: z.array(z.array(z.string())),
  }),
  publish: z.enum(["never_attempted", "ambiguous", "rejected", "accepted"]),
  wake: z.unknown(),
  createdAt: z.number().optional(),
  updatedAt: z.number().optional(),
});

const blobSchema = z.object({
  version: z.literal(OUTBOX_VERSION),
  entries: z.array(z.unknown()),
});

/**
 * A wake whose inputs still agree with the comment it belongs to, or null. A
 * wake that does not is dropped rather than trusted: it is rebuilt from the
 * context on the next attempt, and the relay deduplicates it by feedback
 * revision whatever its timestamp.
 */
function parseWake(
  value: unknown,
  context: ReviewFeedbackContext,
  feedbackArtifactId: string,
  signed: RelayEvent,
): ReviewWakeInput | null {
  const parsed = wakeSchema.safeParse(value);
  if (!parsed.success) return null;
  const wake = parsed.data;
  const consistent =
    wake.content.trim().length > 0 &&
    wake.channelId === context.revision.channelId &&
    wake.parentEventId === context.parentEventId &&
    wake.rootEventId === context.rootEventId &&
    wake.executiveAgentPubkey === context.executiveAgentPubkey &&
    wake.feedbackArtifactId === feedbackArtifactId &&
    wake.feedbackRevisionEventId === signed.id &&
    wake.reviewedArtifactId === context.revision.artifactId &&
    wake.reviewedRevisionEventId === context.revision.eventId &&
    wake.expectedRelayUrl === context.expectedRelayUrl &&
    wake.expectedSignerPubkey === context.expectedSignerPubkey;
  return consistent ? wake : null;
}

/**
 * Validate one stored record. A record only survives if the signed event is
 * exactly what its own context composes (so a record cannot be pointed at a
 * different revision, digest, block, or comment) and was signed by the signer
 * the record is filed under.
 */
export function parseOutboxEntry(
  value: unknown,
  now: number,
): OutboxEntry | null {
  const parsed = entrySchema.safeParse(value);
  if (!parsed.success) return null;
  const { feedbackArtifactId, publish } = parsed.data;
  const signed = parsed.data.signed as RelayEvent;
  const context = parsed.data.context as unknown as ReviewFeedbackContext;
  if (signed.pubkey !== context.expectedSignerPubkey) return null;
  try {
    const block = validateReviewBlock({
      id: context.block.id,
      title: context.block.title,
      sourceRef: context.block.sourceRef,
    });
    const template = buildFeedbackArtifact({
      feedbackArtifactId,
      revision: context.revision,
      block,
      request: normalizeFeedbackRequest(context.request),
      viewport: context.viewport,
    });
    if (
      signed.kind !== template.kind ||
      signed.content !== template.content ||
      JSON.stringify(signed.tags) !== JSON.stringify(template.tags)
    ) {
      return null;
    }
  } catch {
    return null;
  }
  return {
    context,
    feedbackArtifactId,
    signed,
    publish,
    wake: parseWake(parsed.data.wake, context, feedbackArtifactId, signed),
    createdAt: parsed.data.createdAt ?? now,
    updatedAt: parsed.data.updatedAt ?? now,
  };
}

/** Whether the relay provably holds nothing for this entry. */
function isAbandonable(entry: OutboxEntry): boolean {
  return entry.publish === "never_attempted" || entry.publish === "rejected";
}

function parseBlob(
  raw: string | null,
  signerPubkey: string,
  now: number,
): OutboxEntry[] {
  if (!raw) return [];
  let json: unknown;
  try {
    json = JSON.parse(raw);
  } catch {
    return [];
  }
  const blob = blobSchema.safeParse(json);
  if (!blob.success) return [];
  const signer = signerPubkey.toLowerCase();
  const entries: OutboxEntry[] = [];
  for (const candidate of blob.data.entries) {
    const entry = parseOutboxEntry(candidate, now);
    if (!entry || entry.context.expectedSignerPubkey !== signer) continue;
    const idle = now - entry.updatedAt;
    if (isAbandonable(entry) && idle > OUTBOX_ABANDONABLE_TTL_SECONDS) continue;
    entries.push(entry);
    if (entries.length >= OUTBOX_MAX_ENTRIES) break;
  }
  return entries;
}

// ── The outbox ──────────────────────────────────────────────────────────────

function outboxKey(signerPubkey: string): string {
  return `${OUTBOX_STORAGE_PREFIX}${signerPubkey.toLowerCase()}`;
}

const TOO_MANY = `Buzz keeps at most ${OUTBOX_MAX_ENTRIES} unfinished comments on this device. Finish or discard one before adding another.`;
const TAKEN =
  "This block already has a comment waiting to finish. Finish it before writing another.";
const NOT_DURABLE =
  "This comment could not be saved on this device, so nothing was sent. Free some storage and try again.";

/**
 * Build the outbox over `storage`. `now` is wall-clock seconds (injected so
 * retention is testable).
 */
export function createReviewOutbox(
  storage: OutboxStorage,
  now: () => number = () => Math.floor(Date.now() / 1000),
): ReviewOutbox {
  // The webview re-reads this on every render; reparsing is skipped while the
  // stored text is unchanged.
  let cache: {
    key: string;
    raw: string | null;
    entries: OutboxEntry[];
  } | null = null;

  function read(signerPubkey: string): OutboxEntry[] {
    const key = outboxKey(signerPubkey);
    let raw: string | null = null;
    try {
      raw = storage.getItem(key);
    } catch {
      // Unreadable storage holds no recoverable entry.
    }
    if (cache && cache.key === key && cache.raw === raw) return cache.entries;
    const entries = parseBlob(raw, signerPubkey, now());
    cache = { key, raw, entries };
    return entries;
  }

  function write(signerPubkey: string, entries: OutboxEntry[]) {
    const key = outboxKey(signerPubkey);
    if (entries.length === 0) {
      try {
        storage.removeItem(key);
      } catch (cause) {
        throw new ReviewOutboxError("unwritable", NOT_DURABLE, { cause });
      }
      cache = { key, raw: null, entries: [] };
      return;
    }
    const raw = JSON.stringify({ version: OUTBOX_VERSION, entries });
    try {
      storage.setItem(key, raw);
    } catch (cause) {
      throw new ReviewOutboxError("unwritable", NOT_DURABLE, { cause });
    }
    cache = { key, raw, entries };
  }

  const indexOf = (
    entries: OutboxEntry[],
    identity: ReviewSubmissionIdentity,
  ) => {
    const key = identityKey(identity);
    return entries.findIndex(
      (entry) => identityKey(identityOf(entry.context)) === key,
    );
  };

  /**
   * Room for one more comment, and no overwriting of one the relay may hold.
   * `index` is the entry already filed under the same identity, or -1.
   */
  function assertCanStore(
    entries: OutboxEntry[],
    index: number,
    feedbackArtifactId: string,
  ) {
    const existing = entries[index];
    if (
      existing &&
      existing.feedbackArtifactId !== feedbackArtifactId &&
      !isAbandonable(existing)
    ) {
      // A different comment may already be on the relay for this block: it
      // must be finished, never silently replaced.
      throw new ReviewOutboxError("taken", TAKEN);
    }
    if (index === -1 && entries.length >= OUTBOX_MAX_ENTRIES) {
      throw new ReviewOutboxError("full", TOO_MANY);
    }
  }

  return {
    assertRoom(identity, feedbackArtifactId) {
      const entries = read(identity.signerPubkey);
      assertCanStore(entries, indexOf(entries, identity), feedbackArtifactId);
    },

    load(identity) {
      const entries = read(identity.signerPubkey);
      return entries[indexOf(entries, identity)];
    },

    save(record) {
      const identity = identityOf(record.context);
      const entries = [...read(identity.signerPubkey)];
      const index = indexOf(entries, identity);
      const existing = entries[index];
      assertCanStore(entries, index, record.feedbackArtifactId);
      const stamp = now();
      const entry: OutboxEntry = {
        ...record,
        createdAt:
          existing && existing.feedbackArtifactId === record.feedbackArtifactId
            ? existing.createdAt
            : stamp,
        updatedAt: stamp,
      };
      if (JSON.stringify(entry).length > OUTBOX_MAX_RECORD_CHARS) {
        throw new ReviewOutboxError(
          "too-large",
          "This comment is too large to keep safely on this device, so nothing was sent.",
        );
      }
      if (index === -1) entries.push(entry);
      else entries[index] = entry;
      write(identity.signerPubkey, entries);
    },

    remove(identity) {
      const entries = read(identity.signerPubkey);
      const index = indexOf(entries, identity);
      if (index === -1) return;
      write(
        identity.signerPubkey,
        entries.filter((_, position) => position !== index),
      );
    },

    list(scope) {
      return read(scope.signerPubkey).filter(
        (entry) =>
          entry.context.expectedRelayUrl === scope.relayUrl &&
          entry.context.revision.channelId === scope.channelId &&
          entry.context.revision.artifactId === scope.artifactId,
      );
    },
  };
}
