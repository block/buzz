import { useMemo } from "react";
import type { ReactNode } from "react";
import { ChevronRight, FileCode2, RefreshCw } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery } from "@/features/channels/hooks";
import {
  formatFullDateTime,
  formatShortMonthDay,
} from "@/features/messages/lib/dateFormatters";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { Alert } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { PageHeader } from "@/shared/ui/PageHeader";
import { useReviewInboxQuery } from "../hooks";
import type { ReviewRevision } from "../lib/reviewContract";
import { reviewSearchFromRevision } from "../lib/reviewRoute";

function ReviewInboxRow({
  channelName,
  onOpen,
  review,
}: {
  channelName: string;
  onOpen: () => void;
  review: ReviewRevision;
}) {
  return (
    <button
      aria-label={`Open ${review.title} from ${channelName}`}
      className="group flex w-full min-w-0 items-center gap-4 rounded-2xl border border-border/70 bg-background px-4 py-3 text-start transition-colors hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      onClick={onOpen}
      type="button"
    >
      <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-muted text-muted-foreground group-hover:text-foreground">
        <FileCode2 aria-hidden="true" className="h-5 w-5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {review.title}
        </span>
        <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
          <span>{channelName}</span>
          <span aria-hidden="true">·</span>
          <span>Revision {review.revision}</span>
          <span aria-hidden="true">·</span>
          <time
            dateTime={new Date(review.createdAt * 1_000).toISOString()}
            title={formatFullDateTime(review.createdAt)}
          >
            {formatShortMonthDay(review.createdAt)}
          </time>
        </span>
      </span>
      <ChevronRight
        aria-hidden="true"
        className="h-4 w-4 shrink-0 text-muted-foreground"
      />
    </button>
  );
}

/** Persistent Canvas destination listing the active relay's current reviews. */
export function ReviewInboxScreen() {
  const { goReview } = useAppNavigation();
  const channelsQuery = useChannelsQuery();
  const inboxQuery = useReviewInboxQuery();
  const channelNames = useMemo(
    () =>
      new Map(
        (channelsQuery.data ?? []).map((channel) => [channel.id, channel.name]),
      ),
    [channelsQuery.data],
  );
  const refreshUnavailable = inboxQuery.isFetching;

  let body: ReactNode;
  if (inboxQuery.isLoading) {
    body = (
      <p className="py-12 text-center text-sm text-muted-foreground">
        Loading reviews…
      </p>
    );
  } else if (inboxQuery.isError) {
    body = (
      <Alert variant="destructive">
        <p>The Canvas inbox could not be loaded.</p>
        <Button
          className="mt-2"
          onClick={() => void inboxQuery.refetch()}
          size="sm"
          type="button"
          variant="outline"
        >
          Try again
        </Button>
      </Alert>
    );
  } else if (!inboxQuery.data?.reviews.length) {
    body = (
      <div className="flex flex-1 flex-col items-center justify-center rounded-2xl border border-dashed border-border/70 px-6 py-16 text-center">
        <FileCode2
          aria-hidden="true"
          className="h-8 w-8 text-muted-foreground"
        />
        <p className="mt-3 text-sm font-medium">No reviews yet</p>
        <p className="mt-1 max-w-md text-sm text-muted-foreground">
          HTML reviews published to this community will stay available here.
        </p>
      </div>
    );
  } else {
    body = (
      <ul aria-label="HTML reviews" className="space-y-2">
        {inboxQuery.data.reviews.map((review) => (
          <li key={`${review.channelId}:${review.artifactId}`}>
            <ReviewInboxRow
              channelName={
                channelNames.get(review.channelId) ?? review.channelId
              }
              onOpen={() =>
                void goReview(
                  review.channelId,
                  review.artifactId,
                  reviewSearchFromRevision(review),
                )
              }
              review={review}
            />
          </li>
        ))}
      </ul>
    );
  }

  return (
    <div
      className="flex h-full min-h-0 min-w-0 flex-1 flex-col"
      data-testid="review-inbox"
    >
      <TopChromeInsetHeader className="px-6 py-5">
        <PageHeader
          action={
            <Button
              aria-disabled={refreshUnavailable || undefined}
              aria-label="Refresh reviews"
              className="aria-disabled:pointer-events-none aria-disabled:opacity-50"
              onClick={() => {
                if (!refreshUnavailable) void inboxQuery.refetch();
              }}
              size="icon"
              type="button"
              variant="ghost"
            >
              <RefreshCw
                aria-hidden="true"
                className={refreshUnavailable ? "animate-spin" : undefined}
              />
            </Button>
          }
          description="Review generated HTML and send block-level feedback."
          title="Canvas"
        />
      </TopChromeInsetHeader>
      <main className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        {inboxQuery.data?.rejected || inboxQuery.data?.truncated ? (
          <Alert className="mb-4">
            {inboxQuery.data.rejected
              ? `${inboxQuery.data.rejected} invalid review ${inboxQuery.data.rejected === 1 ? "was" : "were"} withheld. `
              : null}
            {inboxQuery.data.truncated
              ? "Only the 1,000 newest reviews are shown."
              : null}
          </Alert>
        ) : null}
        {body}
      </main>
    </div>
  );
}
