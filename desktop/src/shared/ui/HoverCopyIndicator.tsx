import { Check, Copy } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { cn } from "@/shared/lib/cn";

export function useCopyFeedback({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const resetTimerRef = React.useRef<number | null>(null);

  React.useEffect(
    () => () => {
      if (resetTimerRef.current !== null) {
        window.clearTimeout(resetTimerRef.current);
      }
    },
    [],
  );

  const copy = React.useCallback(async () => {
    try {
      await writeTextToClipboard(value);
      setCopied(true);
      if (resetTimerRef.current !== null) {
        window.clearTimeout(resetTimerRef.current);
      }
      resetTimerRef.current = window.setTimeout(() => {
        setCopied(false);
        resetTimerRef.current = null;
      }, 1_500);
      toast.success(`Copied ${label.toLowerCase()}`);
    } catch {
      toast.error(`Couldn't copy ${label.toLowerCase()}.`);
    }
  }, [label, value]);

  return { copied, copy };
}

export function HoverCopyIndicator({
  copied,
  testId,
}: {
  copied: boolean;
  testId?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative h-4 w-4 shrink-0 transition-opacity duration-150 ease-out motion-reduce:transition-none group-hover:opacity-100 group-focus-within:opacity-100",
        copied ? "opacity-100" : "opacity-0",
      )}
      data-copied={copied}
      data-testid={testId}
    >
      <Copy
        className={cn(
          "absolute inset-0 h-4 w-4 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
          copied ? "scale-95 opacity-0" : "scale-100 opacity-100",
        )}
      />
      <Check
        className={cn(
          "absolute inset-0 h-4 w-4 text-primary transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
          copied ? "scale-100 opacity-100" : "scale-95 opacity-0",
        )}
      />
    </span>
  );
}
