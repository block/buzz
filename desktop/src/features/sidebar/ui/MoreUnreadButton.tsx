import type * as React from "react";

import { Button } from "@/shared/ui/button";

const MORE_UNREAD_BUTTON_CLASS =
  "h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-[11px] font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-3.5";

export function MoreUnreadButton({
  count,
  icon,
  onClick,
  testId,
}: {
  count: number;
  icon: React.ReactNode;
  onClick: () => void;
  testId: string;
}) {
  return (
    <div className="flex justify-center py-1">
      <Button
        className={MORE_UNREAD_BUTTON_CLASS}
        data-testid={testId}
        onClick={onClick}
        size="sm"
        type="button"
        variant="outline"
      >
        {icon}
        {count} more unread
      </Button>
    </div>
  );
}
