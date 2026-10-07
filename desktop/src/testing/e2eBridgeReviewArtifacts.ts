import type { RelayEvent } from "@/shared/api/types";

/**
 * Mock-mode NIP-AR review artifact store. It mirrors the observable contract
 * of the native commands (`list_review_artifacts`,
 * `get_review_artifact_revision`, `list_review_feedback`,
 * `fetch_review_document`, `send_artifact_feedback_message`) so E2E specs
 * exercise the real webview
 * pipeline: query → parse → hash-verified document → sandboxed frame →
 * signed feedback → wake message. Signature, envelope, and hash verification
 * themselves are proven by the native Rust tests; the document hash check is
 * repeated here because the webview re-checks it too.
 */

type ReviewSendArgs = {
  channelId: string;
  content: string;
  parentEventId: string;
  rootEventId: string;
  executiveAgentPubkey: string;
  feedbackArtifactId: string;
  feedbackRevisionEventId: string;
  reviewedArtifactId: string;
  reviewedRevisionEventId: string;
  createdAt: number;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
};

type SendChannelMessage = (args: {
  channelId: string;
  content: string;
  parentEventId: string;
  rootEventId: string;
  mentionPubkeys: string[];
  mediaTags: string[][];
}) => Promise<unknown>;

/** The relay's freshness window for an event timestamp, in seconds. */
const MAX_TIMESTAMP_DRIFT_SECONDS = 900;
const REVIEW_ARTIFACTS_STORAGE_KEY = "buzz-e2e-review-artifacts.v1";

type StoredReviewArtifacts = {
  revisions: Array<[string, RelayEvent[]]>;
  documents: Array<[string, string]>;
};

function readStoredReviewArtifacts(): StoredReviewArtifacts {
  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(REVIEW_ARTIFACTS_STORAGE_KEY) ?? "{}",
    ) as Partial<StoredReviewArtifacts>;
    return {
      revisions: Array.isArray(parsed.revisions) ? parsed.revisions : [],
      documents: Array.isArray(parsed.documents) ? parsed.documents : [],
    };
  } catch {
    return { revisions: [], documents: [] };
  }
}

function tagValue(event: RelayEvent, name: string): string | undefined {
  return event.tags.find((tag) => tag[0] === name)?.[1];
}

async function sha256Hex(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function createMockReviewArtifacts() {
  const stored = readStoredReviewArtifacts();
  /** Revision history per artifact UUID, oldest first. */
  const revisions = new Map<string, RelayEvent[]>(stored.revisions);
  /** Document HTML by the SHA-256 the revision *declares* (not its real hash). */
  const documents = new Map<string, string>(stored.documents);
  const persistArtifacts = () => {
    window.localStorage.setItem(
      REVIEW_ARTIFACTS_STORAGE_KEY,
      JSON.stringify({
        revisions: [...revisions],
        documents: [...documents],
      } satisfies StoredReviewArtifacts),
    );
  };
  const feedback: RelayEvent[] = [];
  let failNextPublish: string | null = null;
  let failNextNotification: string | null = null;
  let loseNextAck: string | null = null;
  let loseNextPublishAck: string | null = null;
  let headReadFailure: string | null = null;
  let reconcileFailure: string | null = null;
  /** Simulated time elapsed on the relay and the client since the page loaded. */
  let clockOffsetSeconds = 0;
  const relayNowSeconds = () =>
    Math.floor(Date.now() / 1000) + clockOffsetSeconds;
  /**
   * The relay's feedback-wake ledger: one recorded wake per feedback revision
   * (the author is always this reviewer), with its bindings and the timestamp
   * of the wake that was stored.
   */
  const wakeLedger = new Map<
    string,
    { bindings: string; createdAt: number; sent: unknown }
  >();
  const wakeAttempts: Array<{
    createdAt: number;
    outcome: "stale" | "stored" | "duplicate";
  }> = [];

  /**
   * Mirrors the relay's atomic admission rule: feedback is create-only and its
   * `target_revision` must be the current, undeleted review head of the named
   * artifact with the claimed Synaxis artifact and payload digest.
   */
  const feedbackRejection = (event: RelayEvent): string | null => {
    if (tagValue(event, "op") !== "create") {
      return "invalid: synaxis feedback is create-only";
    }
    if (
      feedback.some(
        (existing) => tagValue(existing, "d") === tagValue(event, "d"),
      )
    ) {
      return "conflict: artifact identity is taken";
    }
    let reviewed: Record<string, unknown> | undefined;
    try {
      reviewed = JSON.parse(event.content).reviewed;
    } catch {
      return "invalid: feedback content is not JSON";
    }
    const history = revisions.get(String(reviewed?.buzz_artifact_id));
    const head = history?.at(-1);
    if (
      !head ||
      head.id !== reviewed?.buzz_revision_event_id ||
      tagValue(head, "h") !== tagValue(event, "h")
    ) {
      return "conflict: feedback target is not the current review revision";
    }
    const review = JSON.parse(head.content);
    if (
      review.artifact?.id !== reviewed?.synaxis_artifact_id ||
      review.artifact?.payload_digest !== reviewed?.payload_digest
    ) {
      return "invalid: feedback binding does not match the review revision";
    }
    return null;
  };

  return {
    /** The mock relay's (and client's) clock in seconds, including any advance. */
    nowSeconds: relayNowSeconds,

    /** Test seeding: publish a review revision (and optionally its document). */
    publishRevision(input: { event: RelayEvent; html?: string }) {
      const artifactId = tagValue(input.event, "d");
      if (!artifactId) throw new Error("mock revision needs a d tag");
      revisions.set(artifactId, [
        ...(revisions.get(artifactId) ?? []),
        input.event,
      ]);
      const declared = input.event.tags
        .find((tag) => tag[0] === "imeta")
        ?.find((part) => part.startsWith("x "))
        ?.slice(2);
      if (declared && input.html !== undefined) {
        documents.set(declared, input.html);
      }
      persistArtifacts();
      return null;
    },

    /** Test seeding: make the next feedback revision publish fail. */
    failNextPublish(message: string) {
      failNextPublish = message;
      return null;
    },

    /** Test seeding: make the next wake message fail. */
    failNextNotification(message: string) {
      failNextNotification = message;
      return null;
    },

    /** Test seeding: the relay accepts the next wake, but its ack is lost. */
    loseNextNotificationAck(message: string) {
      loseNextAck = message;
      return null;
    },

    /**
     * Test seeding: the relay stores the next feedback revision, but its
     * acknowledgement never reaches the client (so the client sees a failure).
     */
    loseNextPublishAck(message: string) {
      loseNextPublishAck = message;
      return null;
    },

    /** Test seeding: every head/revision read fails until cleared with null. */
    failHeadReads(message: string | null) {
      headReadFailure = message;
      return null;
    },

    /** The mock relay accepting a kind-45010 EVENT published over the socket. */
    acceptArtifactEvent(event: RelayEvent): { ok: boolean; message: string } {
      if (failNextPublish) {
        const message = failNextPublish;
        failNextPublish = null;
        return { ok: false, message };
      }
      const type = tagValue(event, "type");
      if (type === "synaxis.artifact-feedback") {
        // Resubmitting an already accepted event succeeds without re-applying
        // it, and before any freshness check (so a lost acknowledgement is
        // recoverable at any age).
        if (!feedback.some((existing) => existing.id === event.id)) {
          if (
            Math.abs(event.created_at - relayNowSeconds()) >
            MAX_TIMESTAMP_DRIFT_SECONDS
          ) {
            return {
              ok: false,
              message: "invalid: event timestamp too far from server time",
            };
          }
          const rejection = feedbackRejection(event);
          if (rejection) return { ok: false, message: rejection };
          feedback.push(event);
          if (loseNextPublishAck) {
            const message = loseNextPublishAck;
            loseNextPublishAck = null;
            return { ok: false, message };
          }
        }
      } else if (type === "synaxis.html-review") {
        this.publishRevision({ event });
      }
      return { ok: true, message: "" };
    },

    getRevision(args: {
      channelId: string;
      artifactId: string;
      revisionEventId?: string | null;
    }) {
      if (headReadFailure) throw new Error(headReadFailure);
      const history = revisions.get(args.artifactId);
      const current = history?.at(-1);
      if (!history || !current) {
        throw new Error("review artifact revision not found");
      }
      const event = args.revisionEventId
        ? history.find((candidate) => candidate.id === args.revisionEventId)
        : current;
      if (!event) throw new Error("review artifact revision not found");
      if (tagValue(event, "h") !== args.channelId) {
        throw new Error("artifact revision belongs to a different channel");
      }
      return { event, currentEventId: current.id };
    },

    listArtifacts() {
      const events = [...revisions.values()]
        .map((history) => history.at(-1))
        .filter((event): event is RelayEvent => event !== undefined)
        .sort(
          (left, right) =>
            right.created_at - left.created_at ||
            left.id.localeCompare(right.id),
        );
      return { events, rejected: 0, truncated: false };
    },

    listFeedback(args: {
      channelId: string;
      targetRevisionIds: string[];
      feedbackEventIds: string[];
    }) {
      const events = feedback.filter(
        (event) =>
          tagValue(event, "h") === args.channelId &&
          (args.targetRevisionIds.includes(
            tagValue(event, "target_revision") ?? "",
          ) ||
            args.feedbackEventIds.includes(event.id)),
      );
      return { events, rejected: 0, truncated: false };
    },

    /** Mirrors the native size + SHA-256 refusal before bytes reach the webview. */
    async fetchDocument(args: {
      url: string;
      expectedSha256: string;
      expectedSize: number;
    }): Promise<ArrayBuffer> {
      const html = documents.get(args.expectedSha256);
      if (html === undefined) throw new Error("relay returned 404 Not Found");
      const bytes = new TextEncoder().encode(html);
      if (bytes.length !== args.expectedSize) {
        throw new Error(
          `size mismatch: fetched ${bytes.length} bytes but ${args.expectedSize} were declared`,
        );
      }
      if ((await sha256Hex(bytes)) !== args.expectedSha256) {
        throw new Error(
          "hash mismatch: fetched bytes do not match the declared SHA-256",
        );
      }
      const buffer = new ArrayBuffer(bytes.length);
      new Uint8Array(buffer).set(bytes);
      return buffer;
    },

    /**
     * The native wake command against the mock relay. It mirrors the real
     * layers: the relay admits a wake only for feedback it holds, answers the
     * wake it already recorded before its freshness check, refuses any other
     * timestamp more than 900 s from its clock as stale, and keeps ONE wake per
     * feedback revision (a later wake with the same bindings is a
     * `duplicate:`). The command retries once with a fresh timestamp only
     * after such a stale refusal.
     */
    async sendFeedbackMessage(
      args: ReviewSendArgs,
      sendChannelMessage: SendChannelMessage,
    ) {
      if (!args.expectedRelayUrl || !args.expectedSignerPubkey) {
        throw new Error(
          "feedback notification needs its captured relay and signer",
        );
      }
      if (failNextNotification) {
        const message = failNextNotification;
        failNextNotification = null;
        throw new Error(message);
      }
      if (
        !feedback.some((event) => event.id === args.feedbackRevisionEventId)
      ) {
        throw new Error("feedback revision was not published");
      }
      const bindings = JSON.stringify([
        args.channelId,
        args.feedbackArtifactId,
        args.reviewedArtifactId,
        args.reviewedRevisionEventId,
        args.rootEventId,
        args.parentEventId,
        args.executiveAgentPubkey,
      ]);
      const attempt = async (createdAt: number) => {
        const recorded = wakeLedger.get(args.feedbackRevisionEventId);
        // The recorded wake itself is answered before the freshness check.
        const isRecordedWake = recorded?.createdAt === createdAt;
        if (
          !isRecordedWake &&
          Math.abs(createdAt - relayNowSeconds()) > MAX_TIMESTAMP_DRIFT_SECONDS
        ) {
          wakeAttempts.push({ createdAt, outcome: "stale" });
          return "stale" as const;
        }
        if (recorded) {
          if (recorded.bindings !== bindings) {
            throw new Error(
              "invalid: a wake for this feedback revision already exists with different bindings",
            );
          }
          wakeAttempts.push({ createdAt, outcome: "duplicate" });
          return recorded.sent;
        }
        const sent = await sendChannelMessage({
          channelId: args.channelId,
          content: args.content,
          parentEventId: args.parentEventId,
          rootEventId: args.rootEventId,
          mentionPubkeys: [args.executiveAgentPubkey],
          // The real command appends these two tags natively.
          mediaTags: [
            ["feedback", args.feedbackArtifactId, args.feedbackRevisionEventId],
            ["artifact", args.reviewedArtifactId, args.reviewedRevisionEventId],
          ],
        });
        wakeLedger.set(args.feedbackRevisionEventId, {
          bindings,
          createdAt,
          sent,
        });
        wakeAttempts.push({ createdAt, outcome: "stored" });
        return sent;
      };

      let sent = await attempt(args.createdAt);
      if (sent === "stale") {
        // Refused before anything was stored, and the exact frozen event is not
        // held: rebuild once with a fresh timestamp.
        const fresh = relayNowSeconds();
        sent = fresh === args.createdAt ? "stale" : await attempt(fresh);
        if (sent === "stale") {
          throw new Error(
            "relay returned 400 Bad Request: invalid: event timestamp too far from server time",
          );
        }
      }
      if (loseNextAck) {
        const message = loseNextAck;
        loseNextAck = null;
        throw new Error(message);
      }
      return sent;
    },

    /** Test seeding: pretend `seconds` passed on the relay and the client. */
    advanceClock(seconds: number) {
      clockOffsetSeconds += seconds;
      return null;
    },

    /** What the relay was asked to take as a wake, and what it did with it. */
    listWakeAttempts() {
      return {
        attempts: [...wakeAttempts],
        stored: wakeLedger.size,
      };
    },

    /** Test seeding: reading the relay for an exact event fails until null. */
    failReconcile(message: string | null) {
      reconcileFailure = message;
      return null;
    },

    /**
     * The native exact-event read: held only when the relay stores this very
     * feedback event. Rejects (never answers "absent") when it cannot read.
     */
    reconcileFeedback(args: {
      event: RelayEvent;
      channelId: string;
      feedbackArtifactId: string;
      expectedRelayUrl: string;
      expectedSignerPubkey: string;
    }) {
      if (!args.expectedRelayUrl || !args.expectedSignerPubkey) {
        throw new Error(
          "feedback reconciliation needs its captured relay and signer",
        );
      }
      if (reconcileFailure) throw new Error(reconcileFailure);
      const held = feedback.some(
        (event) =>
          event.id === args.event.id &&
          tagValue(event, "d") === args.feedbackArtifactId &&
          tagValue(event, "h") === args.channelId,
      );
      return { status: held ? "held" : "absent" };
    },
  };
}
