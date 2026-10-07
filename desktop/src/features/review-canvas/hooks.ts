import {
  type QueryClient,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  fetchReviewDocument,
  getReviewArtifactRevision,
  listReviewArtifacts,
  listReviewFeedback,
} from "@/shared/api/tauriArtifacts";
import {
  bindFeedbackToReview,
  parseReviewFeedbackList,
  parseReviewRevision,
  type ReviewBlock,
  ReviewContractError,
  type ReviewFeedback,
  type ReviewRevision,
} from "./lib/reviewContract";
import { loadReviewDocument } from "./lib/reviewDocumentLoader";

/** How often an open review re-reads the artifact's current head. */
export const REVIEW_HEAD_REFRESH_MS = 30_000;
/** Earlier revisions a feedback listing may resolve to verify dispositions. */
const MAX_HISTORICAL_REVISIONS = 8;

export type ReviewInboxResult = {
  reviews: ReviewRevision[];
  rejected: number;
  truncated: boolean;
};

async function readReviewInbox(): Promise<ReviewInboxResult> {
  const listing = await listReviewArtifacts();
  const reviews: ReviewRevision[] = [];
  let rejected = listing.rejected;
  for (const event of listing.events) {
    try {
      reviews.push(parseReviewRevision(event));
    } catch (error) {
      if (!(error instanceof ReviewContractError)) throw error;
      rejected += 1;
    }
  }
  reviews.sort(
    (left, right) =>
      right.createdAt - left.createdAt ||
      left.eventId.localeCompare(right.eventId),
  );
  return { reviews, rejected, truncated: listing.truncated };
}

/** Current reviews available to the persistent Canvas inbox. */
export function useReviewInboxQuery() {
  return useQuery({
    queryKey: ["review-artifacts"],
    queryFn: readReviewInbox,
    retry: 1,
    staleTime: REVIEW_HEAD_REFRESH_MS,
    refetchOnWindowFocus: true,
  });
}

type RevisionInput = {
  channelId: string;
  artifactId: string;
  /** Exact revision to open; the current head when omitted. */
  revisionEventId: string | undefined;
  /** False while the route identity is malformed and nothing may be fetched. */
  enabled?: boolean;
};

/** One verified, parsed revision plus the artifact's current head event ID. */
async function readReviewRevision(input: {
  channelId: string;
  artifactId: string;
  revisionEventId: string | undefined;
}) {
  const result = await getReviewArtifactRevision(input);
  const revision = parseReviewRevision(result.event);
  if (
    revision.channelId !== input.channelId ||
    revision.artifactId !== input.artifactId ||
    (input.revisionEventId && revision.eventId !== input.revisionEventId)
  ) {
    throw new ReviewContractError(
      "The relay returned a different artifact than the one requested.",
    );
  }
  return { revision, currentEventId: result.currentEventId };
}

/**
 * One verified review revision plus the artifact's current head ID. A pinned
 * revision re-reads the head on an interval and on window focus so a newer
 * revision shows up without reloading the canvas.
 */
export function useReviewRevisionQuery(input: RevisionInput) {
  const { channelId, artifactId, revisionEventId } = input;
  return useQuery({
    queryKey: [
      "review-artifact",
      channelId,
      artifactId,
      revisionEventId ?? "current",
    ],
    queryFn: () =>
      readReviewRevision({ channelId, artifactId, revisionEventId }),
    enabled: input.enabled ?? true,
    retry: 1,
    // An unpinned open resolves once; only a pinned revision watches the head.
    refetchInterval: revisionEventId ? REVIEW_HEAD_REFRESH_MS : false,
    refetchOnWindowFocus: true,
  });
}

/**
 * Authoritative, uncached head check run immediately before a new feedback
 * event is signed (never to gate retrying one already signed). Any failure to
 * prove the revision is the current head (a newer revision, a deleted review, a
 * relay error) rejects, so a new comment fails closed.
 */
export async function assertReviewHeadCurrent(
  revision: ReviewRevision,
): Promise<void> {
  const head = await readReviewRevision({
    channelId: revision.channelId,
    artifactId: revision.artifactId,
    revisionEventId: undefined,
  });
  if (head.currentEventId !== revision.eventId) {
    throw new ReviewContractError(
      "A newer revision of this review exists, so this feedback was not sent. Open the latest revision to comment.",
    );
  }
}

function loadDocumentQuery(revision: ReviewRevision) {
  return {
    queryKey: ["review-document", revision.eventId, revision.document.sha256],
    queryFn: () => loadReviewDocument(revision, fetchReviewDocument),
    // The bytes are content-addressed by a signed hash and never change.
    staleTime: Number.POSITIVE_INFINITY,
  };
}

/** The verified, parsed, sanitized document of one review revision. */
export function useReviewDocumentQuery(revision: ReviewRevision | undefined) {
  return useQuery({
    queryKey: [
      "review-document",
      revision?.eventId ?? "none",
      revision?.document.sha256 ?? "none",
    ],
    queryFn: () => {
      if (!revision) throw new ReviewContractError("No review revision.");
      return loadDocumentQuery(revision).queryFn();
    },
    enabled: Boolean(revision),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

export type BoundReviewFeedback = {
  feedback: ReviewFeedback;
  /** The trusted block from the review's own table, never event metadata. */
  block: ReviewBlock;
  /** The revision the feedback was written against. */
  revision: ReviewRevision;
};

export type ReviewFeedbackResult = {
  /** Feedback verified against its reviewed revision and document, newest first. */
  items: BoundReviewFeedback[];
  /** Entries that failed verification or the binding and were withheld. */
  withheld: number;
  /** The relay holds more feedback than was read: the oldest is absent. */
  truncated: boolean;
};

type RevisionContext = { revision: ReviewRevision; blocks: ReviewBlock[] };

/** An earlier revision (and its declared blocks) a disposition points back to. */
async function resolveHistoricalRevision(
  queryClient: QueryClient,
  channelId: string,
  artifactId: string,
  revisionEventId: string,
): Promise<RevisionContext> {
  const { revision } = await queryClient.fetchQuery({
    queryKey: ["review-artifact", channelId, artifactId, revisionEventId],
    queryFn: () =>
      readReviewRevision({ channelId, artifactId, revisionEventId }),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const document = await queryClient.fetchQuery(loadDocumentQuery(revision));
  return { revision, blocks: document.blocks };
}

/**
 * Feedback attached to this revision, plus the earlier feedback its
 * dispositions name. The relay never delivers artifact revisions over the
 * timeline subscription, so feedback is read through the artifact query, and
 * every entry is joined to the exact revision and declared block it signed.
 * Entries that do not bind are withheld, not shown with event-supplied text.
 */
export function useReviewFeedbackQuery(
  channelId: string,
  revision: ReviewRevision | undefined,
  blocks: ReviewBlock[] | undefined,
) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: ["review-feedback", channelId, revision?.eventId ?? "none"],
    queryFn: async (): Promise<ReviewFeedbackResult> => {
      if (!revision || !blocks) {
        throw new ReviewContractError("No review revision.");
      }
      const listing = await listReviewFeedback({
        channelId,
        targetRevisionIds: [revision.eventId],
        feedbackEventIds: revision.dispositions.map(
          (disposition) => disposition.feedbackRevisionEventId,
        ),
      });
      const parsed = parseReviewFeedbackList(listing.events);
      let withheld = parsed.invalid + listing.rejected;
      const contexts = new Map<string, RevisionContext | null>([
        [revision.eventId, { revision, blocks }],
      ]);
      const items: BoundReviewFeedback[] = [];
      for (const feedback of parsed.feedback) {
        const targetId = feedback.reviewed.buzzRevisionEventId;
        if (
          feedback.reviewed.buzzArtifactId !== revision.artifactId ||
          (!contexts.has(targetId) && contexts.size > MAX_HISTORICAL_REVISIONS)
        ) {
          withheld += 1;
          continue;
        }
        if (!contexts.has(targetId)) {
          contexts.set(
            targetId,
            await resolveHistoricalRevision(
              queryClient,
              channelId,
              revision.artifactId,
              targetId,
            ).catch(() => null),
          );
        }
        const context = contexts.get(targetId);
        try {
          if (!context) throw new ReviewContractError("Revision unavailable.");
          items.push({
            feedback,
            block: bindFeedbackToReview(
              feedback,
              context.revision,
              context.blocks,
            ),
            revision: context.revision,
          });
        } catch (error) {
          if (!(error instanceof ReviewContractError)) throw error;
          withheld += 1;
        }
      }
      return { items, withheld, truncated: listing.truncated };
    },
    enabled: Boolean(revision && blocks),
    retry: 1,
  });
}
