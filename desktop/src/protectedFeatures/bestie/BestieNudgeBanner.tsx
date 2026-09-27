import { Bell } from "lucide-react";

import { Button } from "@/shared/ui/button";
import {
  clearBestieNudge,
  useBestieNudge,
  type BestieNudge,
} from "./bestieNudgeStore";

export function BestieNudgeBanner({
  onDismiss,
  onOpenList,
}: {
  onDismiss?: () => void;
  onOpenList?: () => void;
}) {
  const nudge = useBestieNudge();
  if (!nudge) return null;
  return (
    <BestieNudgeBannerView
      nudge={nudge}
      onDismiss={onDismiss}
      onOpenList={onOpenList}
    />
  );
}

export function BestieNudgeBannerView({
  nudge,
  onDismiss,
  onOpenList,
}: {
  nudge: BestieNudge;
  onDismiss?: () => void;
  onOpenList?: () => void;
}) {
  return (
    <div
      className="flex shrink-0 items-start gap-2 rounded-xl border border-amber-500/40 bg-amber-500/15 px-3 py-2 text-amber-950 dark:text-amber-100"
      data-testid="bestie-nudge-banner"
      role="status"
    >
      <Bell aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <p className="text-xs font-semibold">{nudge.title}</p>
        <p className="mt-0.5 text-xs leading-snug opacity-90">{nudge.body}</p>
        <p className="mt-1 text-2xs opacity-70">
          Proactive Bestie check-in — not a normal DM.
        </p>
      </div>
      <div className="flex shrink-0 flex-col gap-1">
        {onOpenList ? (
          <Button
            className="h-6 px-2 text-2xs"
            data-testid="bestie-nudge-open-list"
            onClick={onOpenList}
            size="xs"
            type="button"
            variant="secondary"
          >
            View list
          </Button>
        ) : null}
        <Button
          className="h-6 px-2 text-2xs"
          data-testid="bestie-nudge-dismiss"
          onClick={() => {
            clearBestieNudge();
            onDismiss?.();
          }}
          size="xs"
          type="button"
          variant="ghost"
        >
          Dismiss
        </Button>
      </div>
    </div>
  );
}
