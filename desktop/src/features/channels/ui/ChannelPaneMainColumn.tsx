import type * as React from "react";

import { useChannelViewOverride } from "@/features/channels/ui/ChannelViewOverrideContext";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";

const IN_FLOW_CHANNEL_CONTENT_STYLE = {
  "--buzz-channel-content-top-padding": "0rem",
  "--channel-top-chrome-height": "0.25rem",
} as React.CSSProperties;

export function ChannelPaneMainColumn({
  children,
  hideRightHeader = false,
}: {
  children: React.ReactNode;
  hideRightHeader?: boolean;
}) {
  const channelView = useChannelViewOverride();
  const mainColumnHeader = channelView?.mainColumnHeader;
  const headerOnRight =
    Boolean(mainColumnHeader) &&
    channelView?.mainColumnHeaderPlacement === "right";
  const className = cn(
    "relative isolate flex min-h-0 min-w-0 flex-1 flex-col",
    channelView?.mainContent && "hidden",
  );

  if (!mainColumnHeader) return <div className={className}>{children}</div>;

  return (
    <div className={className}>
      <div
        className={cn(
          "relative flex min-h-0 min-w-0 flex-1 flex-col",
          channelChrome.contentPadding,
        )}
      >
        <div
          className={cn(
            "relative flex min-h-0 min-w-0 flex-1",
            headerOnRight
              ? "flex-row [container-type:inline-size]"
              : "flex-col",
          )}
          style={IN_FLOW_CHANNEL_CONTENT_STYLE}
        >
          <div
            className={cn(
              headerOnRight
                ? cn(
                    "order-last min-h-0 shrink-0 [width:clamp(24rem,38%,28.8rem)]",
                    hideRightHeader
                      ? "hidden"
                      : "hidden [@container(min-width:52rem)]:flex",
                  )
                : "contents",
            )}
            data-testid="channel-main-column-header"
          >
            {mainColumnHeader}
          </div>
          <div
            className={cn(
              channelView?.hideMainColumnBody
                ? "hidden"
                : headerOnRight
                  ? "relative flex min-h-0 min-w-0 flex-1 flex-col"
                  : "contents",
            )}
            data-testid="channel-main-column-body"
          >
            {children}
          </div>
        </div>
      </div>
    </div>
  );
}

export function ChannelPaneMainContent() {
  const mainContent = useChannelViewOverride()?.mainContent;
  if (!mainContent) return null;

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden pt-(--buzz-channel-content-top-padding,5.75rem)"
      data-testid="channel-main-content"
    >
      {mainContent}
    </div>
  );
}
