import type * as React from "react";

import { cn } from "@/shared/lib/cn";

type MessageHeaderRowProps = {
  children: React.ReactNode;
  className?: string;
};

export function MessageHeaderRow({
  children,
  className,
}: MessageHeaderRowProps) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0 leading-4",
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * Divider between two pieces of message-header metadata.
 *
 * The header runs several independent facts together on one line, and without a
 * divider they read as one phrase: an agent message came out as "managed by You
 * 9:53 AM". A middot is what the rest of the app already uses for this
 * (`MessageThreadSummaryRow`, project rows, the mention list).
 *
 * `aria-hidden` because the divider is punctuation for the eye only — the header
 * already reads as separate nodes to a screen reader, and `MessageAgentOwner`
 * supplies its own "Agent managed by" label.
 *
 * No horizontal margin: `MessageHeaderRow` is a flex row with `gap-x-1.5`, so
 * spacing comes from the container. Adding margin here double-spaces it.
 */
export function MessageMetaSeparator() {
  return (
    <span aria-hidden="true" className="text-xs text-muted-foreground/40">
      ·
    </span>
  );
}

type MessageAuthorTextProps = {
  as?: "div" | "h3" | "span";
  children: React.ReactNode;
  className?: string;
  hoverUnderline?: boolean;
};

export function MessageAuthorText({
  as: Component = "span",
  children,
  className,
  hoverUnderline = false,
}: MessageAuthorTextProps) {
  return (
    <Component
      className={cn(
        "truncate text-sm font-semibold leading-4 tracking-tight",
        hoverUnderline && "hover:underline",
        className,
      )}
      data-testid="message-author"
    >
      {children}
    </Component>
  );
}
