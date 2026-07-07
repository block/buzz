import type * as React from "react";
import {
  MessageScroller as MessageScrollerPrimitive,
  useMessageScroller,
  useMessageScrollerScrollable,
  useMessageScrollerVisibility,
} from "@shadcn/react/message-scroller";
import { ArrowDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

function MessageScrollerProvider(
  props: React.ComponentProps<typeof MessageScrollerPrimitive.Provider>,
) {
  return <MessageScrollerPrimitive.Provider {...props} />;
}

function MessageScroller({
  className,
  children,
  topFade = false,
  ...props
}: React.ComponentProps<typeof MessageScrollerPrimitive.Root> & {
  /**
   * Fade content out as it scrolls under the header above the scroller —
   * the same treatment the sidebar's pinned header applies to channels.
   */
  topFade?: boolean;
}) {
  return (
    <MessageScrollerPrimitive.Root
      className={cn(
        "group/message-scroller relative flex size-full min-h-0 flex-col overflow-hidden",
        className,
      )}
      data-slot="message-scroller"
      {...props}
    >
      {children}
      {/* Edge fades as overlays instead of a mask-image on the scrolling
          viewport — masks on scroll containers force per-frame repaints in
          WKWebView and make scrolling visibly choppy. */}
      {topFade ? (
        <div
          aria-hidden
          className="pointer-events-none absolute inset-x-0 top-0 h-8 bg-gradient-to-b from-background to-transparent"
        />
      ) : null}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-b from-transparent to-background"
      />
    </MessageScrollerPrimitive.Root>
  );
}

function MessageScrollerViewport({
  className,
  ...props
}: React.ComponentProps<typeof MessageScrollerPrimitive.Viewport>) {
  return (
    <MessageScrollerPrimitive.Viewport
      className={cn(
        "buzz-sidebar-scrollbar size-full min-h-0 min-w-0 overflow-y-auto overscroll-contain",
        className,
      )}
      data-slot="message-scroller-viewport"
      {...props}
    />
  );
}

function MessageScrollerContent({
  className,
  ...props
}: React.ComponentProps<typeof MessageScrollerPrimitive.Content>) {
  return (
    <MessageScrollerPrimitive.Content
      className={cn("flex h-max min-h-full flex-col gap-6", className)}
      data-slot="message-scroller-content"
      {...props}
    />
  );
}

function MessageScrollerItem({
  className,
  scrollAnchor = false,
  ...props
}: React.ComponentProps<typeof MessageScrollerPrimitive.Item>) {
  // No content-visibility:auto here: skipped items report estimated sizes,
  // which makes scrollHeight jump during streaming and breaks the
  // scroller's at-bottom detection (stick-to-bottom randomly disengages).
  return (
    <MessageScrollerPrimitive.Item
      className={cn("min-w-0 shrink-0", className)}
      data-slot="message-scroller-item"
      scrollAnchor={scrollAnchor}
      {...props}
    />
  );
}

function MessageScrollerButton({
  className,
  children,
  direction = "end",
  render,
  ...props
}: React.ComponentProps<typeof MessageScrollerPrimitive.Button>) {
  return (
    <MessageScrollerPrimitive.Button
      className={cn(
        "absolute inset-x-1/2 z-10 -translate-x-1/2 border-border bg-background text-foreground shadow-sm transition-[translate,scale,opacity] duration-200 hover:bg-muted hover:text-foreground data-[active=false]:pointer-events-none data-[active=false]:scale-95 data-[active=false]:opacity-0 data-[active=false]:duration-300 data-[active=true]:translate-y-0 data-[active=true]:scale-100 data-[active=true]:opacity-100 data-[direction=end]:bottom-4 data-[direction=end]:data-[active=false]:translate-y-full data-[direction=start]:top-4 data-[direction=start]:data-[active=false]:-translate-y-full data-[direction=start]:[&_svg]:rotate-180",
        className,
      )}
      data-direction={direction}
      data-slot="message-scroller-button"
      direction={direction}
      render={render ?? <Button size="icon-xs" variant="secondary" />}
      {...props}
    >
      {children ?? (
        <>
          <ArrowDown aria-hidden />
          <span className="sr-only">
            {direction === "end" ? "Scroll to end" : "Scroll to start"}
          </span>
        </>
      )}
    </MessageScrollerPrimitive.Button>
  );
}

export {
  MessageScrollerProvider,
  MessageScroller,
  MessageScrollerViewport,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerButton,
  useMessageScroller,
  useMessageScrollerScrollable,
  useMessageScrollerVisibility,
};
