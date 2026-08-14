import type * as React from "react";

import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type TopChromeInsetHeaderProps = React.ComponentProps<"div"> & {
  /** Keep the header flush with its parent content row. */
  flush?: boolean;
  /** Render without a local backdrop when a parent supplies shared chrome. */
  transparent?: boolean;
};

/**
 * Flowed header row with the standard chrome backdrop and separators. The
 * global top chrome now sits above this content in normal layout flow.
 */
export function TopChromeInsetHeader({
  className,
  children,
  flush = false,
  transparent = false,
  ...props
}: TopChromeInsetHeaderProps) {
  return (
    <div
      className={cn(
        transparent ? "relative z-40 shrink-0" : topChromeInset.headerBase,
        !flush && topChromeInset.padding,
        !flush && topChromeInset.divider,
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
