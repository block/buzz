import {
  applyOwnerJournalOverride,
  type MissionJournal,
  type NormalizedActivityEvent,
} from "./activityLedger";

/** A signature-verified owner artifact returned by the Tauri authority store. */
export type ValidatedJournalAuthorityArtifact = {
  ownerPubkey: string;
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
};

function validSourceEventId(value: string | null | undefined): value is string {
  return typeof value === "string" && /^[0-9a-f]{64}$/i.test(value);
}

/**
 * Bind verification to both the receipted work and the observer frame that
 * carries the journal-level correlation. Those can be separate signed relay
 * events when the observer batch flushes between turn start and tool output.
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
  const hasCorrelationEvidence =
    correlationId === journal.id || validSourceEventId(correlationSourceId);

  return {
    sourceEventIds: [
      ...new Set(
        [correlationSourceId, ...receiptedSourceIds].filter(validSourceEventId),
      ),
    ].sort(),
    hasReceiptedEvidence: receiptedSourceIds.length > 0,
    hasCorrelationEvidence,
  };
}

/**
 * Overlay owner authority without rewriting the observed source journal.
 *
 * The backend verifies artifact ids, signatures, signer identity, tags, and
 * revision ordering before returning these values. The frontend additionally
 * requires every verification source id to belong to this exact journal, so a
 * valid owner signature for one turn cannot promote a different turn.
 */
export function applyValidatedJournalAuthority(
  journal: MissionJournal,
  artifacts: readonly ValidatedJournalAuthorityArtifact[],
): MissionJournal {
  const matching = artifacts
    .filter(
      (artifact) =>
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

  const sourceEventIds = new Set(
    journal.events
      .map((event) => event.provenance.sourceEventId)
      .filter((id): id is string => Boolean(id)),
  );
  const latestVerification = matching
    .filter(
      (artifact) =>
        artifact.artifactType === "verification" &&
        Boolean(artifact.receiptRef?.trim()) &&
        artifact.sourceEventIds.length > 0 &&
        artifact.sourceEventIds.every((id) => sourceEventIds.has(id)),
    )
    .at(-1);
  if (!latestVerification) return result;

  const timestamp = new Date(
    latestVerification.createdAt * 1_000,
  ).toISOString();
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
      seq:
        Math.max(0, ...journal.events.map((event) => event.provenance.seq)) + 1,
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
