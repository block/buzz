import type * as React from "react";

import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type TopChromeInsetHeaderProps = React.ComponentProps<"div">;

/**
 * Flowed header row that clears the global search/drag chrome and draws the
 * horizontal separator at the bottom edge of that inset.
 */
export function TopChromeInsetHeader({
  className,
  children,
  ...props
}: TopChromeInsetHeaderProps) {
  return (
    <div
      className={cn(
        topChromeInset.headerBase,
        topChromeInset.padding,
        topChromeInset.divider,
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
