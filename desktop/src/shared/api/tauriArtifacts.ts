import { invokeTauri } from "@/shared/api/tauri";
import type { RawSendChannelMessageResult } from "@/shared/api/tauriMessageTypes";
import type { RelayEvent, SendChannelMessageResult } from "@/shared/api/types";

/**
 * A NIP-AR revision the native layer already verified: event ID, Schnorr
 * signature, the complete NIP-AR envelope, exact channel/artifact identity,
 * and artifact type. Nothing here has been interpreted as review content.
 */
export type VerifiedReviewRevision = {
  event: RelayEvent;
  /** Event ID of the artifact's current revision. */
  currentEventId: string;
};

/**
 * Verified feedback revisions plus how many returned events were withheld.
 * `truncated` means the relay holds more feedback than one listing reads (the
 * native layer pages it, newest first, up to a fixed cap), so the oldest
 * comments are absent.
 */
export type VerifiedReviewFeedback = {
  events: RelayEvent[];
  rejected: number;
  truncated: boolean;
};

/**
 * Read a review artifact's current head, and optionally one exact revision,
 * over the relay's HTTP artifact query. Revisions never appear in the ordinary
 * timeline subscription, so this is the only read path.
 */
export async function getReviewArtifactRevision(input: {
  channelId: string;
  artifactId: string;
  revisionEventId?: string | null;
}): Promise<VerifiedReviewRevision> {
  return invokeTauri<VerifiedReviewRevision>("get_review_artifact_revision", {
    channelId: input.channelId,
    artifactId: input.artifactId,
    revisionEventId: input.revisionEventId ?? null,
  });
}

/**
 * List verified `synaxis.artifact-feedback` revisions attached to reviewed
 * revisions, and/or named explicitly by a later revision's dispositions.
 */
export async function listReviewFeedback(input: {
  channelId: string;
  targetRevisionIds: string[];
  feedbackEventIds: string[];
}): Promise<VerifiedReviewFeedback> {
  return invokeTauri<VerifiedReviewFeedback>("list_review_feedback", input);
}

/**
 * Fetch one review document. The native command returns bytes only when the
 * declared size, SHA-256, and UTF-8 encoding all agree.
 */
export async function fetchReviewDocument(input: {
  url: string;
  expectedSha256: string;
  expectedSize: number;
}): Promise<Uint8Array<ArrayBuffer>> {
  const buffer = await invokeTauri<ArrayBuffer>("fetch_review_document", input);
  return new Uint8Array(buffer);
}

/**
 * Publish the kind-9 reply that wakes the executive agent for one already
 * published feedback revision. Fails closed when the active community or
 * identity no longer matches the captured scope. If the relay refuses the
 * frozen timestamp as stale, the native layer retries once with a fresh one;
 * the relay stores at most one wake per feedback revision either way.
 */
export async function sendArtifactFeedbackMessage(input: {
  channelId: string;
  content: string;
  parentEventId: string;
  rootEventId: string;
  executiveAgentPubkey: string;
  feedbackArtifactId: string;
  feedbackRevisionEventId: string;
  reviewedArtifactId: string;
  reviewedRevisionEventId: string;
  /** Frozen with the draft so a retry rebuilds the byte-identical wake event. */
  createdAt: number;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
}): Promise<SendChannelMessageResult> {
  const response = await invokeTauri<RawSendChannelMessageResult>(
    "send_artifact_feedback_message",
    {
      ...input,
      expectedRelayUrl: input.expectedRelayUrl,
      expectedSignerPubkey: input.expectedSignerPubkey,
    },
  );
  return {
    eventId: response.event_id,
    parentEventId: response.parent_event_id,
    rootEventId: response.root_event_id,
    depth: response.depth,
    createdAt: response.created_at,
  };
}

/**
 * Ask the relay, over its writer-authoritative artifact query, whether it
 * already holds exactly this signed feedback revision (same event ID, author,
 * timestamp, tags, and content). Used to settle a publish whose outcome is
 * unknown before the event is resent or its draft is discarded. The native
 * command fails closed when the active community or identity no longer
 * matches the captured scope, and rejects when the relay cannot answer;
 * "absent" is an answer, not a failure.
 */
export async function reconcileReviewFeedbackEvent(input: {
  event: RelayEvent;
  channelId: string;
  feedbackArtifactId: string;
  expectedRelayUrl: string;
  expectedSignerPubkey: string;
}): Promise<"held" | "absent"> {
  const { event } = input;
  const response = await invokeTauri<{ status: "held" | "absent" }>(
    "reconcile_review_feedback_event",
    {
      ...input,
      event: {
        id: event.id,
        pubkey: event.pubkey,
        created_at: event.created_at,
        kind: event.kind,
        tags: event.tags,
        content: event.content,
        sig: event.sig,
      },
    },
  );
  return response.status;
}
