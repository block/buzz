import type * as React from "react";

import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type AuxiliaryPanelHeaderProps = React.ComponentProps<"div"> & {
  /** Render header content without its own backdrop for a shared parent chrome. */
  transparent?: boolean;
};
type AuxiliaryPanelHeaderGroupProps = React.ComponentProps<"div">;
type AuxiliaryPanelTitleProps = React.ComponentProps<"h2">;

/** Compact title/action row for right auxiliary panels in split layouts. */
export function AuxiliaryPanelHeader({
  className,
  children,
  transparent = false,
  ...props
}: AuxiliaryPanelHeaderProps) {
  return (
    <div
      className={cn(
        "pointer-events-none relative z-40 overflow-visible",
        transparent
          ? "bg-transparent"
          : "bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        channelChrome.negativeMargin,
        className,
      )}
      {...props}
    >
      <div
        className="pointer-events-auto relative z-40 shrink-0 cursor-default select-none px-5 py-2"
        data-tauri-drag-region
      >
        <div className="flex h-9 min-w-0 items-center gap-2.5">{children}</div>
      </div>
    </div>
  );
}

export const auxiliaryPanelContentPaddingClass = channelChrome.contentPadding;

export function AuxiliaryPanelHeaderGroup({
  className,
  children,
  ...props
}: AuxiliaryPanelHeaderGroupProps) {
  return (
    <div
      className={cn("flex min-w-0 flex-1 items-center gap-1.5", className)}
      {...props}
    >
      {children}
    </div>
  );
}

export function AuxiliaryPanelTitle({
  className,
  children,
  ...props
}: AuxiliaryPanelTitleProps) {
  return (
    <h2
      className={cn(
        "min-w-0 flex-1 translate-y-px truncate text-base font-semibold leading-6 tracking-tight",
        className,
      )}
      {...props}
    >
      {children}
    </h2>
  );
}
