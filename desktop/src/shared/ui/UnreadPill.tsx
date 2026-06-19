import { ArrowDown, ArrowUp } from "lucide-react";

import { Button } from "@/shared/ui/button";

const UNREAD_PILL_CLASS =
  "pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-primary/40 bg-primary/10 px-2.5 text-2xs font-medium text-primary shadow-xs backdrop-blur-sm hover:bg-primary/20 [&_svg]:size-4";

export function unreadCountLabel(count: number) {
  return `${count} new message${count === 1 ? "" : "s"}`;
}

export function UnreadPill({
  direction,
  label,
  onClick,
  testId,
}: {
  direction: "up" | "down";
  label: string;
  onClick: () => void;
  testId: string;
}) {
  const Arrow = direction === "up" ? ArrowUp : ArrowDown;
  return (
    <Button
      className={UNREAD_PILL_CLASS}
      data-testid={testId}
      onClick={onClick}
      size="sm"
      type="button"
      variant="outline"
    >
      <Arrow aria-hidden />
      {label}
    </Button>
  );
}
