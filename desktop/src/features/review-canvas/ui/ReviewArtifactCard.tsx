import { FileCode2 } from "lucide-react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { Button } from "@/shared/ui/button";
import type { ReviewNotification } from "../lib/reviewContract";
import { reviewSearchFromNotification } from "../lib/reviewRoute";

type ReviewArtifactCardProps = {
  channelId: string;
  notification: ReviewNotification;
};

/**
 * The timeline card for a recognized review notification. It reads nothing
 * from the network and renders nothing from the artifact: it only offers to
 * open the dedicated Review Canvas for the exact announced revision, carrying
 * the notification's thread so the canvas can return to it.
 */
export function ReviewArtifactCard({
  channelId,
  notification,
}: ReviewArtifactCardProps) {
  const { goReview } = useAppNavigation();

  return (
    <div
      className="my-1 inline-flex max-w-sm items-center gap-3 rounded-2xl border border-border/70 bg-muted/40 px-3 py-2"
      data-testid="review-artifact-card"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-background text-muted-foreground">
        <FileCode2 aria-hidden="true" className="h-4 w-4" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          HTML review
        </span>
        <span className="block text-xs text-muted-foreground">
          Comment on the generated page, block by block.
        </span>
      </span>
      <Button
        data-testid="review-artifact-open"
        onClick={() =>
          void goReview(
            channelId,
            notification.artifactId,
            reviewSearchFromNotification(notification),
          )
        }
        size="sm"
        type="button"
      >
        Open review
      </Button>
    </div>
  );
}
