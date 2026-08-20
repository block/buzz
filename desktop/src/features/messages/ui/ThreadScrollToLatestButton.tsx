import { ArrowDown } from "lucide-react";

import { Button } from "@/shared/ui/button";

export function ThreadScrollToLatestButton({
  isAtBottom,
  newMessageCount,
  onScrollToBottom,
}: {
  isAtBottom: boolean;
  newMessageCount: number;
  onScrollToBottom: () => void;
}) {
  if (isAtBottom) return null;

  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-36 z-50 flex justify-center px-4">
      <Button
        className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-2xs font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
        data-testid="thread-scroll-to-latest"
        onClick={onScrollToBottom}
        size="sm"
        type="button"
        variant="outline"
      >
        <ArrowDown aria-hidden />
        {newMessageCount > 0
          ? `${newMessageCount} new message${newMessageCount === 1 ? "" : "s"}`
          : "Jump to latest"}
      </Button>
    </div>
  );
}
