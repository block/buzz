import { Loader2, RotateCcw, WifiOff } from "lucide-react";

import type { CallBannerModel } from "@/features/meetings/ui/callSessionState";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";

type CallConnectionBannerProps = {
  model: NonNullable<CallBannerModel>;
  onRejoin?: () => void;
  /** A big centered overlay for terminal states vs. a slim top strip for
   * transient ones. */
  variant?: "overlay" | "strip";
};

const TONE_CLASS = {
  info: "border-border bg-card text-foreground",
  warning:
    "border-amber-500/40 bg-amber-500/10 text-amber-900 dark:text-amber-100",
  error:
    "border-destructive/40 bg-destructive/10 text-destructive-foreground dark:text-red-100",
} as const;

/** Presentational only — `CallView` decides which model to pass and when. */
export function CallConnectionBanner({
  model,
  onRejoin,
  variant = "strip",
}: CallConnectionBannerProps) {
  const Icon =
    model.tone === "info" && !model.showRejoin
      ? Loader2
      : model.tone === "error"
        ? WifiOff
        : RotateCcw;

  if (variant === "overlay") {
    return (
      <div
        className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-4 bg-background/85 p-8 text-center backdrop-blur-sm"
        data-testid="meeting-call-banner"
      >
        <Icon
          className={cn(
            "h-8 w-8",
            model.tone === "info" && !model.showRejoin && "animate-spin",
          )}
        />
        <div className="space-y-1">
          <p className="text-base font-medium">{model.title}</p>
          {model.detail ? (
            <p className="max-w-sm text-sm text-muted-foreground">
              {model.detail}
            </p>
          ) : null}
        </div>
        {model.showRejoin && onRejoin ? (
          <Button onClick={onRejoin} size="sm" variant="outline">
            <RotateCcw className="h-4 w-4" />
            Rejoin
          </Button>
        ) : null}
      </div>
    );
  }

  return (
    <div
      className={cn(
        "absolute inset-x-0 top-0 z-20 flex items-center justify-center gap-2 border-b px-4 py-2 text-sm",
        TONE_CLASS[model.tone],
      )}
      data-testid="meeting-call-banner"
    >
      <Icon
        className={cn(
          "h-4 w-4 shrink-0",
          model.tone === "info" && !model.showRejoin && "animate-spin",
        )}
      />
      <span className="font-medium">{model.title}</span>
      {model.detail ? (
        <span className="hidden text-current/80 sm:inline">{model.detail}</span>
      ) : null}
      {model.showRejoin && onRejoin ? (
        <Button className="ml-1" onClick={onRejoin} size="xs" variant="outline">
          Rejoin
        </Button>
      ) : null}
    </div>
  );
}
