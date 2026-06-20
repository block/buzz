import type * as React from "react";

import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type TopChromeInsetHeaderProps = React.ComponentProps<"div"> & {
  /** Drop the top-chrome inset so the header sits flush at y=0. */
  flush?: boolean;
};

/**
 * Flowed header row that clears the global search/drag chrome and draws the
 * horizontal separator at the bottom edge of that inset. Pass `flush` to drop
 * the inset and sit the header at the top (e.g. channel/thread panes).
 */
export function TopChromeInsetHeader({
  className,
  children,
  flush = false,
  ...props
}: TopChromeInsetHeaderProps) {
  return (
    <div
      className={cn(
        topChromeInset.headerBase,
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
