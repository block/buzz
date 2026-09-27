import { Star } from "lucide-react";
import * as React from "react";

import { summarizeThreadRoot } from "@/features/messages/lib/sentFromThread";
import { useThreadStars } from "@/features/sidebar/lib/useThreadStars";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { cn } from "@/shared/lib/cn";

type ThreadStarButtonProps = {
  channelId: string;
  channelName: string;
  currentPubkey?: string;
  /** Thread root message body used to build the sidebar label. */
  rootBody?: string;
  rootId: string;
  testId?: string;
};

/**
 * Thread-panel header control: empty star when unstarred, filled when starred.
 * Persists through identity-scoped local storage (`useThreadStars`).
 */
export function ThreadStarButton({
  channelId,
  channelName,
  currentPubkey,
  rootBody = "",
  rootId,
  testId = "thread-star-button",
}: ThreadStarButtonProps) {
  const { isThreadStarred, toggleThreadStar } = useThreadStars(currentPubkey);
  const starred = isThreadStarred(rootId);

  const handleClick = React.useCallback(() => {
    const title = summarizeThreadRoot(rootBody) ?? "Thread";
    toggleThreadStar({
      rootId,
      channelId,
      title,
      channelName,
    });
  }, [channelId, channelName, rootBody, rootId, toggleThreadStar]);

  const label = starred ? "Unstar thread" : "Star thread";

  return (
    <Tooltip disableHoverableContent>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          aria-pressed={starred}
          className="shrink-0"
          data-testid={testId}
          onClick={handleClick}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Star
            aria-hidden
            className={cn(starred && "fill-current text-amber-500")}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}
