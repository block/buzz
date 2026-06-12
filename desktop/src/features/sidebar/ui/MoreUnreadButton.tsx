import type * as React from "react";

import { Button } from "@/shared/ui/button";

const MORE_UNREAD_BUTTON_CLASS =
  "h-7 min-h-7 gap-1.5 rounded-full border-0 bg-primary px-2.5 text-[11px] font-medium text-primary-foreground shadow-md hover:bg-primary/90 [&_svg]:size-4";

export function MoreUnreadButton({
  bottomClassName = "bottom-0",
  count,
  icon,
  onClick,
  position,
  testId,
}: {
  bottomClassName?: string;
  count: number;
  icon: React.ReactNode;
  onClick: () => void;
  position: "top" | "bottom";
  testId: string;
}) {
  return (
    <div
      className={`pointer-events-none absolute inset-x-0 z-10 flex justify-center py-1 ${position === "top" ? "top-0" : bottomClassName}`}
    >
      <Button
        className={`pointer-events-auto ${MORE_UNREAD_BUTTON_CLASS}`}
        data-testid={testId}
        onClick={onClick}
        size="sm"
        type="button"
        variant="ghost"
      >
        {icon}
        {count} more unread
      </Button>
    </div>
  );
}
