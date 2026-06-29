import * as React from "react";
import { Plus } from "lucide-react";

import { cn } from "@/shared/lib/cn";

type CreateIdentityCardProps = React.ButtonHTMLAttributes<HTMLButtonElement> & {
  ariaLabel: string;
  dataTestId: string;
  label: string;
};

export const CreateIdentityCard = React.forwardRef<
  HTMLButtonElement,
  CreateIdentityCardProps
>(function CreateIdentityCard(
  { ariaLabel, className, dataTestId, label, ...buttonProps },
  ref,
) {
  return (
    <button
      aria-label={ariaLabel}
      className={cn(
        "group relative flex aspect-[4/5] w-full min-w-0 items-center justify-center overflow-hidden rounded-xl border border-dashed border-border/80 bg-transparent text-muted-foreground shadow-xs transition-colors hover:border-border hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      data-testid={dataTestId}
      ref={ref}
      type="button"
      {...buttonProps}
    >
      <span className="flex flex-col items-center justify-center gap-2 text-center">
        <Plus className="h-7 w-7 transition-colors" />
        <span className="text-sm font-medium leading-5">{label}</span>
      </span>
    </button>
  );
});
