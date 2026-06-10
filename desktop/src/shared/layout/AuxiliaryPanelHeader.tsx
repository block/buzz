import type * as React from "react";

import { channelChrome, topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

type AuxiliaryPanelHeaderProps = React.ComponentProps<"div">;

/** Compact title/action row for right auxiliary panels in split layouts. */
export function AuxiliaryPanelHeader({
  className,
  children,
  ...props
}: AuxiliaryPanelHeaderProps) {
  return (
    <div
      className={cn(
        "pointer-events-none relative z-30 bg-background/80 backdrop-blur-md after:absolute after:inset-x-0 after:bottom-0 after:h-px after:bg-border/35 after:content-[''] supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55",
        topChromeInset.padding,
        channelChrome.negativeMargin,
        className,
      )}
      {...props}
    >
      <div
        className="pointer-events-auto relative z-30 flex min-h-8 shrink-0 cursor-default select-none items-center gap-2.5 px-4 py-1.5 sm:pr-3"
        data-tauri-drag-region
      >
        {children}
      </div>
    </div>
  );
}

export const auxiliaryPanelTitleClass = channelChrome.splitPanelTitle;
export const auxiliaryPanelCloseButtonClass =
  channelChrome.splitPanelCloseButton;
export const auxiliaryPanelContentPaddingClass = channelChrome.contentPadding;
