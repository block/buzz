import { ArrowDown, ArrowUp } from "lucide-react";
import type { ReactNode } from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

const UNREAD_PILL_CLASS =
  "pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/70 bg-background/95 px-2 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70 shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4";
const PRIMARY_UNREAD_PILL_CLASS =
  "pointer-events-auto h-7 min-h-7 max-w-[calc(100%_-_1rem)] overflow-hidden gap-1.5 rounded-full px-2 py-1 text-xs font-medium shadow-sm [&_svg]:size-4";

export function unreadCountLabel(count: number) {
  return `${count} new message${count === 1 ? "" : "s"}`;
}

export function UnreadPill({
  accessibleLabel,
  className,
  direction,
  emphasis = "default",
  label,
  leading,
  onClick,
  testId,
}: {
  accessibleLabel?: string;
  className?: string;
  direction: "up" | "down";
  emphasis?: "default" | "primary";
  label: string;
  leading?: ReactNode;
  onClick: () => void;
  testId: string;
}) {
  const Arrow = direction === "up" ? ArrowUp : ArrowDown;
  return (
    <Button
      aria-label={accessibleLabel}
      className={cn(
        emphasis === "primary" ? PRIMARY_UNREAD_PILL_CLASS : UNREAD_PILL_CLASS,
        className,
      )}
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant={emphasis === "primary" ? "default" : "outline"}
    >
      <Arrow aria-hidden />
      {leading}
      <span className="min-w-0 truncate">{label}</span>
    </Button>
  );
}
