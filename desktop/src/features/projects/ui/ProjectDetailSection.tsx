import { ChevronDown } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

/**
 * Expand/collapse block for work-item detail (description, files, activity).
 * The chevron sits immediately after the title. It points down when open
 * and right when collapsed.
 */
export function ProjectDetailSection({
  children,
  className,
  contentClassName,
  count,
  defaultOpen = true,
  headerClassName,
  onOpenChange,
  open: openProp,
  testId,
  title,
}: {
  children: React.ReactNode;
  /** Extra classes for the outer section, e.g. flex growth in fill layouts. */
  className?: string;
  /** Extra classes for the expanded content wrapper. */
  contentClassName?: string;
  count?: number;
  defaultOpen?: boolean;
  /** Extra classes for the header row, e.g. to align it with a reading column while the content is full-bleed. */
  headerClassName?: string;
  onOpenChange?: (open: boolean) => void;
  open?: boolean;
  testId?: string;
  title: string;
}) {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(defaultOpen);
  const isControlled = openProp !== undefined;
  const open = isControlled ? openProp : uncontrolledOpen;
  const setOpen = (next: boolean) => {
    if (!isControlled) setUncontrolledOpen(next);
    onOpenChange?.(next);
  };

  return (
    <section
      className={className}
      data-open={open ? "true" : "false"}
      data-testid={testId ?? "project-detail-section"}
    >
      <button
        aria-expanded={open}
        className={cn(
          "flex min-h-10 w-full min-w-0 items-center gap-2 px-6 py-2 text-left text-sm font-medium leading-5 text-foreground transition-colors hover:bg-muted/20",
          headerClassName,
        )}
        onClick={() => setOpen(!open)}
        type="button"
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="truncate">{title}</span>
          <ChevronDown
            className={cn(
              "h-4 w-4 shrink-0 text-muted-foreground transition-transform",
              !open && "-rotate-90",
            )}
          />
          {open && count != null ? (
            <span className="shrink-0 font-medium text-muted-foreground">
              {count}
            </span>
          ) : null}
        </span>
      </button>
      {open ? (
        <div className={cn("px-6 pb-6 pt-1", contentClassName)}>{children}</div>
      ) : null}
    </section>
  );
}
