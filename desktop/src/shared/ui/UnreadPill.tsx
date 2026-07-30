import { ArrowDown, ArrowUp } from "lucide-react";

import { Button } from "@/shared/ui/button";

const UNREAD_PILL_CLASS =
  "pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/70 bg-background/95 px-2 py-1 text-2xs font-medium tracking-[0.02em] text-muted-foreground/70 shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4";

export function unreadCountLabel(count: number) {
  return `${count} new message${count === 1 ? "" : "s"}`;
}

export function UnreadPill({
  direction,
  label,
  onClick,
  testId,
}: {
  /** `none` renders a static pill without a direction arrow. */
  direction: "up" | "down" | "none";
  label: string;
  onClick: () => void;
  testId: string;
}) {
  const Arrow =
    direction === "up" ? ArrowUp : direction === "down" ? ArrowDown : null;
  return (
    <Button
      className={UNREAD_PILL_CLASS}
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant="outline"
    >
      {Arrow ? <Arrow aria-hidden /> : null}
      {label}
    </Button>
  );
}
