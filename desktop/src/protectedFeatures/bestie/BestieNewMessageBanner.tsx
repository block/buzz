import { MessageSquare } from "lucide-react";

import { Button } from "@/shared/ui/button";
import type { BestiePopoverNewMessageTarget } from "./bestiePopoverNewMessage";

/**
 * Assistant popover New message banner — same card layout as the Reminder
 * (BestieNudge) banner: title, body preview, Dismiss, and View.
 */
export function BestieNewMessageBanner({
  queueLength = 1,
  target,
  onDismiss,
  onView,
}: {
  /** Total pending items including the current target (for "N more"). */
  queueLength?: number;
  target: BestiePopoverNewMessageTarget;
  onDismiss: (target: BestiePopoverNewMessageTarget) => void;
  onView: (target: BestiePopoverNewMessageTarget) => void;
}) {
  const remaining = Math.max(0, queueLength - 1);
  const preview =
    target.preview.trim() ||
    (target.outsideSession ? "New message in another thread" : "New message below");

  return (
    <div
      className="flex shrink-0 items-start gap-2 rounded-xl border border-sky-500/40 bg-sky-500/15 px-3 py-2 text-sky-950 dark:text-sky-100"
      data-testid="bestie-new-message-banner"
      role="status"
    >
      <MessageSquare aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        <p className="text-xs font-semibold">New message</p>
        <p className="mt-0.5 text-xs leading-snug opacity-90">{preview}</p>
        {remaining > 0 ? (
          <p
            className="mt-0.5 text-2xs opacity-70"
            data-testid="bestie-new-message-queue-count"
          >
            {remaining === 1 ? "1 more waiting" : `${remaining} more waiting`}
          </p>
        ) : null}
        <div className="mt-1.5 flex flex-wrap gap-1">
          <Button
            className="h-6 px-2 text-2xs"
            data-testid="bestie-new-message-dismiss"
            onClick={() => onDismiss(target)}
            size="xs"
            type="button"
            variant="ghost"
          >
            Dismiss
          </Button>
          <Button
            className="h-6 px-2 text-2xs"
            data-testid="bestie-new-message-jump"
            onClick={() => onView(target)}
            size="xs"
            type="button"
            variant="secondary"
          >
            View
          </Button>
        </div>
      </div>
    </div>
  );
}
