import * as React from "react";

import { AgentManagementMarker } from "@/features/agents/ui/OtherSetupAgentMarker";
import { UserProfilePopover } from "@/features/profile/ui/UserProfilePopover";

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
        "flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0 leading-message-author",
        className,
      )}
      data-testid="message-header"
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

/**
 * Renders divider-separated header metadata segments in order, skipping empty
 * slots. Each divider is grouped with the segment it precedes so the two wrap
 * together — as loose siblings in a flex-wrap row, a divider can end up alone
 * at the start of the second line.
 *
 * For agent replies, prefer a dedicated primary cluster (name · time · chips)
 * with `shrink-0`, and put "managed by" in a following sibling so it wraps to
 * a second row before time/chips do.
 */
export function MessageMetaSegments({
  segments,
}: {
  segments: ReadonlyArray<{
    key: string;
    node: React.ReactNode;
    /**
     * Keep this segment on one line and do not let flex shrink it below its
     * content width. Used for compact chip groups that must sit beside the
     * timestamp when space remains.
     */
    nowrap?: boolean;
  }>;
}) {
  const present = segments.filter((slot) => Boolean(slot.node));
  return (
    <>
      {present.map(({ key, node, nowrap }, index) =>
        index === 0 ? (
          <React.Fragment key={key}>{node}</React.Fragment>
        ) : (
          <span
            className={cn(
              "inline-flex items-baseline gap-x-1.5",
              nowrap ? "shrink-0" : "min-w-0",
            )}
            key={key}
          >
            <MessageMetaSeparator />
            {node}
          </span>
        ),
      )}
    </>
  );
}

type MessageAuthorTextProps = {
  as?: "div" | "h3" | "span";
  children: React.ReactNode;
  className?: string;
  hoverUnderline?: boolean;
  /**
   * Keep the full display name on one line without ellipsis. Used on the
   * message header primary row so chips do not crush the name into "F…".
   */
  nowrap?: boolean;
};

export function MessageAuthorText({
  as: Component = "span",
  children,
  className,
  hoverUnderline = false,
  nowrap = false,
}: MessageAuthorTextProps) {
  return (
    <Component
      className={cn(
        "text-message font-semibold leading-message-author tracking-normal",
        nowrap ? "whitespace-nowrap" : "truncate",
        hoverUnderline && "hover:underline",
        className,
      )}
      data-testid="message-author"
    >
      {children}
    </Component>
  );
}

/** Author navigation and provenance always refer to the same exact identity. */
export function MessageAuthorIdentity({
  pubkey,
  ownerPubkey,
  role,
  displayName,
  children,
}: {
  pubkey?: string | null;
  ownerPubkey?: string | null;
  role?: string;
  displayName: string;
  children: React.ReactNode;
}) {
  return (
    <>
      {pubkey ? (
        <UserProfilePopover
          pubkey={pubkey}
          role={role}
          botIdenticonValue={displayName}
          triggerClassName="shrink-0"
        >
          <button
            className="whitespace-nowrap rounded leading-message-author focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            type="button"
          >
            {children}
          </button>
        </UserProfilePopover>
      ) : (
        children
      )}
      <AgentManagementMarker pubkey={pubkey} ownerPubkey={ownerPubkey} />
    </>
  );
}
