import {
  applyOwnerJournalOverride,
  type MissionJournal,
  type NormalizedActivityEvent,
} from "./activityLedger";
import { canonicalRelayUrl } from "./managedAgentRuntimeStatus";

/** A signature-verified owner artifact returned by the Tauri authority store. */
export type ValidatedJournalAuthorityArtifact = {
  ownerPubkey: string;
  relayUrl: string;
  eventId: string;
  signature: string;
  createdAt: number;
  artifactType: "owner_override" | "verification";
  journalId: string;
  correlationId: string;
  revision: number;
  summary: string | null;
  note: string | null;
  receiptRef: string | null;
  sourceEventIds: string[];
};

/**
 * Authority artifacts are journal-scoped, even when their supporting evidence
 * is a tool event with its own tool-call correlation.
 */
export function journalAuthorityCorrelationId(
  journal: Pick<MissionJournal, "correlationId">,
): string {
  return journal.correlationId;
}

export type JournalVerificationSources = {
  sourceEventIds: string[];
  hasReceiptedEvidence: boolean;
  hasCorrelationEvidence: boolean;
  hasSupportedSourceSet: boolean;
  overflowCount: number;
};

export const MAX_JOURNAL_VERIFICATION_SOURCE_EVENTS = 256;

function validSourceEventId(value: string | null | undefined): value is string {
  return typeof value === "string" && /^[0-9a-f]{64}$/i.test(value);
}

function isCurrentVerificationEvidence(
  event: NormalizedActivityEvent,
): boolean {
  if (
    event.provenance.sourceKind === 24201 ||
    event.provenance.observerKind === "owner_verification"
  ) {
    return false;
  }
  // Every retained observer frame changes the journal an owner is verifying.
  // A later message, prompt, thought, or plan is still later activity, even
  // when it is only CLAIMED. Requiring its signed source prevents an older
  // receipt from promoting an expanded journal back to VERIFIED.
  return validSourceEventId(event.provenance.sourceEventId);
}

function currentVerificationSourceEventId(
  journal: MissionJournal,
): string | null {
  for (let index = journal.events.length - 1; index >= 0; index -= 1) {
    const event = journal.events[index];
    if (event && isCurrentVerificationEvidence(event)) {
      return event.provenance.sourceEventId;
    }
  }
  return null;
}

/**
 * Bind verification to receipted work and the latest retained observer frame.
 * The stable journal/turn correlation is bound separately by the backend; the
 * source lookup below is a fail-closed fallback for nonstandard journals.
 */
export function journalVerificationSources(
  journal: MissionJournal,
): JournalVerificationSources {
  const correlationId = journalAuthorityCorrelationId(journal);
  const receiptedSourceIds = journal.events
    .filter((event) => event.proofState === "RECEIPTED")
    .map((event) => event.provenance.sourceEventId)
    .filter(validSourceEventId);
  const correlationSourceId = journal.events.find(
    (event) =>
      event.provenance.triggeringEventIds.includes(correlationId) ||
      event.toolCallId === correlationId ||
      event.messageId === correlationId,
  )?.provenance.sourceEventId;
  const currentSourceEventId = currentVerificationSourceEventId(journal);
  const hasCorrelationEvidence =
    correlationId === journal.id || validSourceEventId(correlationSourceId);

  const sourceEventIds = [
    ...new Set(
      [correlationSourceId, ...receiptedSourceIds, currentSourceEventId].filter(
        validSourceEventId,
      ),
    ),
  ].sort();
  return {
    sourceEventIds,
    hasReceiptedEvidence: receiptedSourceIds.length > 0,
    hasCorrelationEvidence,
    hasSupportedSourceSet:
      sourceEventIds.length <= MAX_JOURNAL_VERIFICATION_SOURCE_EVENTS,
    overflowCount: Math.max(
      0,
      sourceEventIds.length - MAX_JOURNAL_VERIFICATION_SOURCE_EVENTS,
    ),
  };
}

/**
 * Overlay owner authority without rewriting the observed source journal.
 *
 * The backend verifies artifact ids, signatures, signer identity, relay scope,
 * revision ordering, and every cited observer source before returning these
 * values. The frontend additionally requires the latest retained evidence to
 * be covered, so older verification cannot overwrite later activity.
 */
export function applyValidatedJournalAuthority(
  journal: MissionJournal,
  artifacts: readonly ValidatedJournalAuthorityArtifact[],
  relayUrl: string,
): MissionJournal {
  const relayScope = canonicalRelayUrl(relayUrl);
  if (!relayScope) return journal;
  const matching = artifacts
    .filter(
      (artifact) =>
        artifact.relayUrl === relayScope &&
        artifact.journalId === journal.id &&
        artifact.correlationId === journal.correlationId,
    )
    .sort(
      (left, right) =>
        left.revision - right.revision ||
        left.createdAt - right.createdAt ||
        left.eventId.localeCompare(right.eventId),
    );

  let result = journal;
  const latestOverride = matching
    .filter(
      (artifact) =>
        artifact.artifactType === "owner_override" &&
        typeof artifact.summary === "string" &&
        artifact.summary.trim().length > 0,
    )
    .at(-1);
  if (latestOverride?.summary) {
    result = applyOwnerJournalOverride(result, {
      summary: latestOverride.summary,
      modifiedAt: new Date(latestOverride.createdAt * 1_000).toISOString(),
      modifiedBy: latestOverride.ownerPubkey,
    });
  }

  const currentSourceEventId = currentVerificationSourceEventId(journal);
  const latestVerification = matching
    .filter(
      (artifact) =>
        artifact.artifactType === "verification" &&
        Boolean(artifact.receiptRef?.trim()) &&
        artifact.sourceEventIds.length > 0 &&
        // The backend revalidates every cited source against this exact
        // owner+relay+journal. A bounded Today window may omit the prior-day
        // correlation root, so require the latest retained source here rather
        // than incorrectly requiring all historical sources to be in memory.
        validSourceEventId(currentSourceEventId) &&
        artifact.sourceEventIds.includes(currentSourceEventId),
    )
    .at(-1);
  if (!latestVerification) return result;

  // A receipt can verify work observed before it was issued, but it cannot
  // erase later terminal evidence. Reapplying authority after a failed or
  // stale-incomplete transition must preserve the journal's fail-closed proof.
  if (result.status === "failed" || result.status === "incomplete") {
    return result;
  }

  const timestamp = new Date(
    latestVerification.createdAt * 1_000,
  ).toISOString();
  let maxSequence = 0;
  for (const event of journal.events) {
    maxSequence = Math.max(maxSequence, event.provenance.seq);
  }
  const verificationEvent: NormalizedActivityEvent = {
    id: latestVerification.eventId,
    journalKey: journal.journalKey,
    correlationId: journal.correlationId,
    category: "status",
    title: "Owner verification",
    detail: latestVerification.receiptRef,
    status: "completed",
    proofState: "VERIFIED",
    timestamp,
    channelId: journal.channelId,
    sessionId: journal.sessionId,
    turnId: journal.turnId,
    toolCallId: null,
    messageId: null,
    provenance: {
      sourceEventId: latestVerification.eventId,
      sourcePubkey: latestVerification.ownerPubkey,
      sourceKind: 24201,
      sourceCreatedAt: latestVerification.createdAt,
      sourceSignature: latestVerification.signature,
      origin: "unknown",
      observerKind: "owner_verification",
      method: null,
      sessionUpdate: null,
      seq: maxSequence + 1,
      timestamp,
      channelId: journal.channelId,
      sessionId: journal.sessionId,
      turnId: journal.turnId,
      toolCallId: null,
      messageId: null,
      triggeringEventIds: latestVerification.sourceEventIds,
    },
    tags: ["owner-signed", "receipt-bound"],
    ownerModifiedAt: timestamp,
    ownerModifiedBy: latestVerification.ownerPubkey,
  };

  return {
    ...result,
    proofState: "VERIFIED",
    status:
      result.status === "ended_unverified" || result.status === "observed"
        ? "completed"
        : result.status,
    claimedCompletionWithoutEvidence: false,
    endedAt:
      Date.parse(timestamp) > Date.parse(result.endedAt)
        ? timestamp
        : result.endedAt,
    eventCount: result.eventCount + 1,
    events: [...result.events, verificationEvent],
  };
}
