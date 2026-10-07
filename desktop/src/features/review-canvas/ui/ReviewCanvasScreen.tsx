import * as React from "react";
import { ArrowLeft, RefreshCw } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Alert } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { useReviewDocumentQuery, useReviewRevisionQuery } from "../hooks";
import { isCanonicalUuid, ReviewContractError } from "../lib/reviewContract";
import { type ReviewRouteSearch, verifyAnnouncement } from "../lib/reviewRoute";
import { ReviewWorkbench } from "./ReviewWorkbench";

type ReviewCanvasScreenProps = {
  channelId: string;
  artifactId: string;
  search: ReviewRouteSearch;
};

function errorText(error: unknown): string {
  return error instanceof Error && error.message
    ? error.message
    : "The review could not be opened.";
}

function ReviewCanvasMessage({
  children,
  tone = "default",
}: {
  children: React.ReactNode;
  tone?: "default" | "destructive";
}) {
  return (
    <div className="p-4">
      <Alert
        data-testid="review-canvas-message"
        role={tone === "destructive" ? "alert" : "status"}
        variant={tone}
      >
        {children}
      </Alert>
    </div>
  );
}

/**
 * The dedicated Review Canvas for one `synaxis.html-review` artifact. Every
 * revision is read through the verified native artifact query; the document is
 * rendered only after its bytes match the signed revision's SHA-256, and the
 * original channel/thread stays one click away.
 */
export function ReviewCanvasScreen({
  channelId,
  artifactId,
  search,
}: ReviewCanvasScreenProps) {
  const { goChannel, goReview } = useAppNavigation();
  const validIdentity =
    isCanonicalUuid(channelId) && isCanonicalUuid(artifactId);
  const revisionQuery = useReviewRevisionQuery({
    channelId,
    artifactId,
    revisionEventId: search.revision,
    enabled: validIdentity,
  });
  const revision = revisionQuery.data?.revision;
  const documentQuery = useReviewDocumentQuery(revision);

  // Pin a "latest" open to the exact revision it resolved, so later refreshes
  // report a newer head instead of silently swapping the document under the
  // reviewer.
  const resolvedRevisionId = revision?.eventId;
  React.useEffect(() => {
    if (!search.revision && resolvedRevisionId) {
      void goReview(
        channelId,
        artifactId,
        { ...search, revision: resolvedRevisionId },
        { replace: true },
      );
    }
  }, [artifactId, channelId, goReview, resolvedRevisionId, search]);

  const backToThread = React.useCallback(() => {
    void goChannel(channelId, {
      messageId: search.notification,
      thread: search.thread,
      threadRootId: search.thread,
    });
  }, [channelId, goChannel, search.notification, search.thread]);

  // The notification's digest and author promise are about the announced
  // revision only; any other revision is verified on its own signature and
  // lineage.
  const openRevision = React.useCallback(
    (revisionEventId: string) => {
      void goReview(channelId, artifactId, {
        ...search,
        revision: revisionEventId,
        digest: undefined,
      });
    },
    [artifactId, channelId, goReview, search],
  );

  const openLatest = React.useCallback(() => {
    const head = revisionQuery.data?.currentEventId;
    if (head) openRevision(head);
  }, [openRevision, revisionQuery.data]);

  // Unavailable, not `disabled`: the head poll flips `isFetching` with no user
  // action, and a focused button that becomes `disabled` loses keyboard focus.
  const refreshUnavailable = revisionQuery.isFetching || !validIdentity;

  let announcementError: string | null = null;
  if (revision) {
    try {
      verifyAnnouncement(revision, search);
    } catch (error) {
      if (!(error instanceof ReviewContractError)) throw error;
      announcementError = error.message;
    }
  }

  let body: React.ReactNode;
  if (!validIdentity) {
    body = (
      <ReviewCanvasMessage tone="destructive">
        This review link is malformed.
      </ReviewCanvasMessage>
    );
  } else if (revisionQuery.isError && !revision) {
    body = (
      <ReviewCanvasMessage tone="destructive">
        <p>{errorText(revisionQuery.error)}</p>
        <Button
          className="mt-2"
          onClick={() => void revisionQuery.refetch()}
          size="sm"
          type="button"
          variant="outline"
        >
          Try again
        </Button>
      </ReviewCanvasMessage>
    );
  } else if (announcementError) {
    body = (
      <ReviewCanvasMessage tone="destructive">
        {announcementError}
      </ReviewCanvasMessage>
    );
  } else if (documentQuery.isError && !documentQuery.data) {
    body = (
      <ReviewCanvasMessage tone="destructive">
        {errorText(documentQuery.error)}
      </ReviewCanvasMessage>
    );
  } else if (revision && documentQuery.data && revisionQuery.data) {
    // Keyed by revision: comment text and the open form belong to one revision
    // and must never carry over when the route switches to another (for
    // example a cached head opened from the stale banner). A signed comment is
    // remembered outside this component per reviewed revision, so it returns
    // only when that same revision is reopened.
    body = (
      <ReviewWorkbench
        key={revision.eventId}
        channelId={channelId}
        currentEventId={revisionQuery.data.currentEventId}
        headChecking={revisionQuery.isFetching}
        headError={revisionQuery.isError}
        document={documentQuery.data}
        onOpenLatest={openLatest}
        onOpenRevision={openRevision}
        revision={revision}
        search={search}
      />
    );
  } else {
    body = (
      <ReviewCanvasMessage>
        <span data-testid="review-canvas-loading">Loading review…</span>
      </ReviewCanvasMessage>
    );
  }

  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-col"
      data-testid="review-canvas"
    >
      <TopChromeInsetHeader data-tauri-drag-region flush>
        <header className="min-w-0 px-5 py-2">
          <div className="flex h-9 min-w-0 items-center gap-2.5">
            <Button
              data-testid="review-canvas-back"
              onClick={backToThread}
              size="sm"
              type="button"
              variant="ghost"
            >
              <ArrowLeft aria-hidden="true" />
              Back to thread
            </Button>
            <h1
              className="min-w-0 flex-1 truncate text-sm font-semibold"
              data-testid="review-canvas-title"
            >
              {revision?.title ?? "HTML review"}
            </h1>
            {revision ? (
              <span
                className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-xs text-muted-foreground"
                data-testid="review-canvas-revision"
              >
                Revision {revision.revision}
              </span>
            ) : null}
            <Button
              aria-disabled={refreshUnavailable || undefined}
              aria-label="Check for a newer revision"
              className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
              onClick={() => {
                if (!refreshUnavailable) void revisionQuery.refetch();
              }}
              size="icon"
              type="button"
              variant="ghost"
            >
              <RefreshCw
                aria-hidden="true"
                className={
                  revisionQuery.isFetching ? "animate-spin" : undefined
                }
              />
            </Button>
          </div>
        </header>
      </TopChromeInsetHeader>
      {body}
    </div>
  );
}
