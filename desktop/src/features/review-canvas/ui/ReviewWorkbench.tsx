import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Alert } from "@/shared/ui/alert";
import { useReviewFeedbackQuery } from "../hooks";
import type { ReviewRouteSearch } from "../lib/reviewRoute";
import type { ReviewRevision } from "../lib/reviewContract";
import {
  buildFrameDocument,
  createFrameNonce,
  type ReviewDocument,
} from "../lib/reviewDocument";
import { ANNOTATOR_ASSET_PATH } from "../lib/reviewAnnotatorProtocol";
import { type ReturnFocus, useReviewComposer } from "../useReviewComposer";
import { ReviewBlockList } from "./ReviewBlockList";
import { ReviewCommentForm } from "./ReviewCommentForm";
import { ReviewFeedbackPanel } from "./ReviewFeedbackPanel";
import { ReviewPendingDelivery } from "./ReviewPendingDelivery";
import {
  ReviewFrame,
  type ReviewFrameHandle,
  type ReviewFrameStatus,
} from "./ReviewFrame";
import { Button } from "@/shared/ui/button";

type ReviewWorkbenchProps = {
  channelId: string;
  revision: ReviewRevision;
  document: ReviewDocument;
  /** Event ID of the artifact's current head. */
  currentEventId: string;
  search: ReviewRouteSearch;
  onOpenLatest: () => void;
  /** Navigate to an exact revision of this artifact by its event ID. */
  onOpenRevision: (revisionEventId: string) => void;
  /** A head read is in flight: sending is paused until it settles. */
  headChecking: boolean;
  /** The last head read failed: sending is paused (fail closed). */
  headError: boolean;
};

/**
 * A verified, parsed review: the sandboxed document on one side and trusted
 * chrome (block list, comment form, feedback) on the other. All reviewer
 * input funnels through `useReviewComposer`.
 */
export function ReviewWorkbench({
  channelId,
  revision,
  document,
  currentEventId,
  search,
  onOpenLatest,
  onOpenRevision,
  headChecking,
  headError,
}: ReviewWorkbenchProps) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const frameRef = React.useRef<ReviewFrameHandle>(null);
  const unavailableAlertRef = React.useRef<HTMLDivElement>(null);
  const [frameStatus, setFrameStatus] =
    React.useState<ReviewFrameStatus>("loading");
  const feedbackQuery = useReviewFeedbackQuery(
    channelId,
    revision,
    document.blocks,
  );

  const stale = currentEventId !== revision.eventId;
  const agent = search.agent ?? null;
  const canComment = !stale && agent !== null;
  // Sending a new comment fails closed: while the head check is in flight or
  // has failed, or a newer revision exists, the form stays open but cannot
  // send, and the composer re-verifies the head immediately before signing.
  // A comment that is already signed is exempt (see `useReviewComposer`):
  // retrying it only re-sends the same event or wake for the relay to settle.
  const blockedReason = stale
    ? "A newer revision of this review exists. Open the latest revision to comment."
    : headError
      ? "Could not confirm this is still the latest revision. Feedback is paused."
      : headChecking
        ? "Checking that this is still the latest revision…"
        : null;

  // The thread the wake message joins: the notification's own thread when the
  // review was opened from it, else the human request that started the work.
  const rootEventId = search.thread ?? revision.attribution.originEventId;
  const parentEventId = search.notification ?? rootEventId;

  // A fresh nonce per composed document; the annotator is a first-party asset.
  // Composition re-parses and audits the finished frame and throws instead of
  // returning anything unexpected, so a document that cannot be composed
  // safely shows a refusal and never renders.
  const frame = React.useMemo(() => {
    try {
      return {
        srcDoc: buildFrameDocument(document, {
          nonce: createFrameNonce(),
          annotatorSrc: new URL(
            `${import.meta.env.BASE_URL}${ANNOTATOR_ASSET_PATH}`,
            window.document.baseURI,
          ).href,
        }),
        error: null,
      };
    } catch (error) {
      return {
        srcDoc: null,
        error:
          error instanceof Error && error.message
            ? error.message
            : "The review document could not be rendered safely.",
      };
    }
  }, [document]);

  const composer = useReviewComposer({
    revision,
    blocks: document.blocks,
    blockedReason,
    executiveAgentPubkey: agent,
    parentEventId,
    rootEventId,
    measureViewport: () =>
      frameRef.current?.measure() ?? { width: 1, height: 1 },
    focusFrameBlock: (id) => frameRef.current?.focusBlock(id) ?? false,
    getScope: () => ({
      relayUrl: activeCommunity?.relayUrl,
      signerPubkey: currentPubkey?.toLowerCase(),
    }),
    onFeedbackSent: () => {
      void queryClient.invalidateQueries({
        queryKey: ["review-feedback", channelId],
      });
    },
  });

  const focusUnavailableNotice = React.useCallback(() => {
    unavailableAlertRef.current?.focus();
  }, []);
  // A block holding a signed draft stays reachable on a stale or unconfirmed
  // head so its accepted feedback can still be retried to the end. The draft
  // carries the agent and thread it was signed for, so reopening the review
  // without a notification does not strand it.
  const pendingBlockIds = new Set(
    composer.pendingDeliveries.map((delivery) => delivery.blockId),
  );
  const canOpenBlock = React.useCallback(
    (id: string) => canComment || pendingBlockIds.has(id),
    [canComment, pendingBlockIds],
  );
  const openFromFrame = React.useCallback(
    (id: string) => {
      if (canOpenBlock(id)) {
        composer.open(id, { kind: "frame", blockId: id });
      } else {
        focusUnavailableNotice();
      }
    },
    [canOpenBlock, composer.open, focusUnavailableNotice],
  );
  const openFromList = React.useCallback(
    (id: string, trigger: HTMLElement) => {
      if (!canOpenBlock(id)) {
        focusUnavailableNotice();
        return;
      }
      const from: ReturnFocus = { kind: "element", element: trigger };
      composer.open(id, from);
    },
    [canOpenBlock, composer.open, focusUnavailableNotice],
  );
  // Reopen a pending comment from the notice; focus returns to the block's
  // entry in the list, which outlives the notice once the comment finishes.
  const openPending = React.useCallback(
    (id: string) => {
      const entry = window.document.getElementById(`review-block-${id}`);
      composer.open(
        id,
        entry
          ? { kind: "element", element: entry }
          : { kind: "frame", blockId: id },
      );
    },
    [composer.open],
  );

  const activeBlock = document.blocks.find(
    (block) => block.id === composer.activeBlockId,
  );
  const feedbackCounts: Record<string, number> = {};
  for (const item of feedbackQuery.data?.items ?? []) {
    if (item.revision.eventId === revision.eventId) {
      feedbackCounts[item.block.id] = (feedbackCounts[item.block.id] ?? 0) + 1;
    }
  }
  // Signed feedback whose agent notification is not confirmed is not
  // delivered, whatever the relay's listing says: the outbox, not the
  // listing, knows a wake is owed (also across a restart, and also when the
  // publish's acknowledgement was lost and the comment is still ambiguous).
  const awaitingNotification = composer.unfinishedFeedbackEventIds;

  return (
    <div
      className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_22rem]"
      data-testid="review-canvas-workbench"
    >
      <div className="relative min-h-[24rem] min-w-0 border-border/60 lg:border-r">
        {frame.srcDoc === null ? (
          <Alert
            className="m-3"
            data-testid="review-frame-refused"
            role="alert"
          >
            {frame.error}
          </Alert>
        ) : (
          <ReviewFrame
            blocks={document.blocks}
            key={revision.eventId}
            onSelectBlock={openFromFrame}
            onStatusChange={setFrameStatus}
            ref={frameRef}
            selectedBlockId={composer.activeBlockId}
            srcDoc={frame.srcDoc}
            title={`Review document: ${revision.title}`}
          />
        )}
      </div>
      <aside
        aria-label="Review controls"
        className="flex min-h-0 flex-col gap-4 overflow-y-auto p-3"
      >
        {stale ? (
          <Alert
            data-testid="review-stale-banner"
            ref={unavailableAlertRef}
            role="status"
            tabIndex={-1}
          >
            <p>
              A newer revision of this review is available. Feedback is accepted
              on the current revision only.
              {composer.pendingDeliveries.length > 0
                ? " A comment you already signed stays with this revision and can still be retried here, now or after you return to it."
                : null}
            </p>
            <Button
              className="mt-2"
              onClick={onOpenLatest}
              size="sm"
              type="button"
            >
              Open the latest revision
            </Button>
          </Alert>
        ) : null}
        {!stale && agent === null ? (
          <Alert ref={unavailableAlertRef} role="status" tabIndex={-1}>
            No executive agent is recorded for this review. Open it from its
            notification in the thread to send feedback.
          </Alert>
        ) : null}
        {!stale && headError ? (
          <Alert data-testid="review-head-error" role="alert">
            Could not confirm this is still the latest revision, so sending new
            feedback is paused. Use “Check for a newer revision” to retry.
          </Alert>
        ) : null}
        {frameStatus === "degraded" ? (
          <Alert role="status">
            The in-page block overlay did not start. Pick a block from the list
            to comment.
          </Alert>
        ) : null}
        <ReviewPendingDelivery
          deliveries={composer.pendingDeliveries.filter(
            (delivery) => delivery.blockId !== composer.activeBlockId,
          )}
          onOpen={openPending}
          onOpenRevision={onOpenRevision}
          otherRevisionDeliveries={composer.otherRevisionDeliveries}
        />

        <section className="flex flex-col gap-2">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Review blocks
          </h2>
          <ReviewBlockList
            blocks={document.blocks}
            disabled={(id) => !canOpenBlock(id)}
            feedbackCounts={feedbackCounts}
            onSelect={openFromList}
            selectedBlockId={composer.activeBlockId}
          />
        </section>

        {activeBlock ? (
          <ReviewCommentForm
            block={activeBlock}
            blockedReason={composer.blockedReason}
            canDiscard={composer.canDiscard}
            feedbackPublished={composer.feedbackPublished}
            locked={composer.locked}
            onCancel={composer.cancel}
            onChange={composer.setText}
            onDiscard={() => void composer.discard()}
            onSubmit={() => void composer.submit()}
            phase={composer.phase}
            publishState={composer.publishState}
            value={composer.text}
          />
        ) : null}

        <ReviewFeedbackPanel
          currentPubkey={currentPubkey}
          awaitingNotification={awaitingNotification}
          failed={feedbackQuery.isError}
          loading={feedbackQuery.isPending}
          result={feedbackQuery.data}
          revision={revision}
        />
        <p
          aria-live="polite"
          className="sr-only"
          data-testid="review-announcer"
          role="status"
        >
          {composer.announcement}
        </p>
      </aside>
    </div>
  );
}
