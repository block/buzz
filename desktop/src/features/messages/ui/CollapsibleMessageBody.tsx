import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  COLLAPSED_MESSAGE_MAX_HEIGHT_PX,
  messageBodyNeedsClamp,
  shouldForceExpandMessageBody,
} from "./collapsibleMessageBody";

type CollapsibleMessageBodyProps = {
  children: React.ReactNode;
  /** Route-target highlight — keep the body fully visible. */
  highlighted?: boolean;
  /** Active timeline search — expand so matches aren't behind the fold. */
  searchQuery?: string;
  className?: string;
};

/**
 * Clamps tall message bodies (agent dumps, long pastes) behind Show more /
 * Show less. Expansion is local to the mounted row and is not persisted.
 */
export function CollapsibleMessageBody({
  children,
  highlighted = false,
  searchQuery,
  className,
}: CollapsibleMessageBodyProps) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [needsClamp, setNeedsClamp] = React.useState(false);
  const [expanded, setExpanded] = React.useState(false);

  const forceExpand = shouldForceExpandMessageBody({
    highlighted,
    searchQuery,
  });
  const isExpanded = forceExpand || expanded;

  React.useLayoutEffect(() => {
    const el = contentRef.current;
    if (!el) return;

    const measure = () => {
      // scrollHeight is the full content height even under max-height.
      setNeedsClamp(messageBodyNeedsClamp(el.scrollHeight));
    };

    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const showToggle = needsClamp && !forceExpand;

  return (
    <div className={cn(className)}>
      <div className="relative">
        <div
          ref={contentRef}
          className={cn(!isExpanded && needsClamp && "overflow-hidden")}
          style={
            !isExpanded && needsClamp
              ? { maxHeight: COLLAPSED_MESSAGE_MAX_HEIGHT_PX }
              : undefined
          }
          data-testid="collapsible-message-body"
          data-collapsed={!isExpanded && needsClamp ? "true" : "false"}
        >
          {children}
        </div>
        {!isExpanded && needsClamp ? (
          <div
            aria-hidden
            className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-linear-to-t from-background to-transparent"
          />
        ) : null}
      </div>
      {showToggle ? (
        <button
          type="button"
          className="mt-1 text-xs font-medium text-primary hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="message-body-expand-toggle"
          onClick={() => setExpanded((value) => !value)}
        >
          {isExpanded ? "Show less" : "Show more"}
        </button>
      ) : null}
    </div>
  );
}
