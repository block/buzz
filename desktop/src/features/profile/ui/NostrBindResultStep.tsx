import * as DialogPrimitive from "@radix-ui/react-dialog";
import { CheckCircle2, TriangleAlert } from "lucide-react";

import {
  getNostrBindResultPresentation,
  type NostrBindResultState,
} from "@/features/profile/lib/nostrBindResult";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";

export function NostrBindResultStep({
  onClose,
  state,
}: {
  onClose: () => void;
  state: NostrBindResultState;
}) {
  const presentation = getNostrBindResultPresentation(state);

  return (
    <>
      <DialogPrimitive.Title
        className="mt-6 text-3xl font-semibold tracking-tight"
        data-testid="nostr-bind-result-title"
      >
        {presentation.title}
      </DialogPrimitive.Title>
      <DialogPrimitive.Description
        className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground"
        id="nostr-bind-description"
      >
        {presentation.description}
      </DialogPrimitive.Description>

      <div
        aria-live="polite"
        className={cn(
          "mt-8 flex w-full items-start gap-3 rounded-2xl border px-4 py-4 text-left shadow-xs",
          presentation.tone === "success" &&
            "border-emerald-500/30 bg-emerald-500/10",
          presentation.tone === "warning" &&
            "border-amber-500/30 bg-amber-500/10",
          presentation.tone === "neutral" && "border-border/70 bg-muted/35",
        )}
        data-result-status={state.status}
        data-testid="nostr-bind-result-status"
        role="status"
      >
        {state.status === "waiting" ? (
          <Spinner
            aria-hidden="true"
            className="mt-0.5 h-5 w-5 shrink-0 border-2"
          />
        ) : state.status === "completed" ? (
          <CheckCircle2
            aria-hidden="true"
            className="mt-0.5 h-5 w-5 shrink-0 text-emerald-600 dark:text-emerald-400"
          />
        ) : (
          <TriangleAlert
            aria-hidden="true"
            className="mt-0.5 h-5 w-5 shrink-0 text-amber-700 dark:text-amber-400"
          />
        )}
        <div className="min-w-0">
          <p className="text-sm font-medium text-foreground">
            {state.status === "waiting"
              ? "Ownership is not confirmed yet."
              : state.status === "completed"
                ? "Ownership is confirmed."
                : "Ownership was not confirmed by Buzz."}
          </p>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">
            {presentation.nextAction}
          </p>
        </div>
      </div>

      <div className="mt-8 flex w-full flex-col gap-2">
        <Button
          className={cn(
            "h-10 w-full",
            state.status === "waiting" &&
              "text-muted-foreground hover:text-accent-foreground",
          )}
          data-testid="nostr-bind-result-close"
          onClick={onClose}
          type="button"
          variant={state.status === "completed" ? "default" : "ghost"}
        >
          {presentation.closeLabel}
        </Button>
        {state.status !== "completed" ? (
          <p className="text-xs leading-5 text-muted-foreground">
            Closing Buzz does not cancel, retry, or change anything in Run402.
          </p>
        ) : null}
      </div>
    </>
  );
}
