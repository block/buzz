import { Video } from "lucide-react";
import * as React from "react";

import {
  agentMediaEntries,
  describeAgentMediaEntries,
} from "@/features/agents/lib/agentMediaEntries";
import { useAgentMediaSessions } from "@/features/agents/lib/useAgentMediaSessions";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

type AgentMediaIndicatorProps = {
  channelId: string;
  className?: string;
  renderMode?: "button" | "menu-item";
};

/**
 * Channel-level presence for a live agent media session (kind:48200).
 *
 * Media is channel presence, like a huddle — not an entry in one agent's work
 * log. It sits beside `HuddleIndicator` for that reason, and deliberately not
 * in the composer activity bar: that bar reports agents *working* (kind:24200
 * observer frames, kind:20002 typing), and an agent on camera is a different
 * claim that should not be filed under the same affordance. The bar is also
 * hidden whenever attention is away from the composer, which is exactly when a
 * live session most needs to be findable.
 *
 * There is no start action, which is the other difference from a huddle: a
 * session is announced by the agent that hosts it, so a member can only join
 * one that already exists. With nothing live this renders nothing.
 *
 * With one agent live, clicking opens its session panel. With several, the
 * control opens a list and every agent gets its own row — a count on its own
 * named a number the member could not act on, since the button opened whichever
 * session was newest and the others had no affordance here at all. Either way
 * the panel is where `AgentMediaSurface` mounts and requests the viewer token.
 * `useOpenAgentActivity` covers both routes: in a channel view an
 * `AgentSessionProvider` opens the panel in place; elsewhere (e.g. the inbox) it
 * navigates to the channel first.
 */
export function AgentMediaIndicator({
  channelId,
  className,
  renderMode = "button",
}: AgentMediaIndicatorProps) {
  const sessions = useAgentMediaSessions(channelId);
  const { openAgentActivity } = useOpenAgentActivity();
  const agentPubkeys = React.useMemo(
    () => Array.from(new Set(sessions.map((session) => session.agentPubkey))),
    [sessions],
  );
  // Named agents read far better than truncated keys, and the batch query is
  // disabled on an empty list — so a channel with no session costs no request.
  const profilesQuery = useUsersBatchQuery(agentPubkeys);
  const profiles = profilesQuery.data?.profiles;

  const entries = React.useMemo(
    () =>
      agentMediaEntries(sessions, (agentPubkey) =>
        resolveUserLabel({ pubkey: agentPubkey, profiles }),
      ),
    [profiles, sessions],
  );

  const open = React.useCallback(
    (agentPubkey: string) => {
      openAgentActivity(agentPubkey, { channelId });
    },
    [channelId, openAgentActivity],
  );

  if (entries.length === 0) {
    return null;
  }

  const description = describeAgentMediaEntries(entries);
  const sole = entries.length === 1 ? entries[0] : null;

  const renderRows = (itemClassName?: string) =>
    entries.map((entry) => (
      <DropdownMenuItem
        className={itemClassName}
        data-testid="channel-agent-media-item"
        key={entry.agentPubkey}
        onSelect={() => open(entry.agentPubkey)}
      >
        <Video />
        <span>{`${entry.label} is live`}</span>
      </DropdownMenuItem>
    ));

  // Already inside the channel actions menu: contribute rows straight to it
  // rather than nesting a second menu, so one live agent costs one click here
  // too. A fragment rather than a wrapper, so the items stay direct children of
  // the menu content that styles and navigates them.
  if (renderMode === "menu-item") {
    return <>{renderRows(className)}</>;
  }

  const trigger = (
    <Button
      aria-label={
        sole
          ? `Open live video — ${description}`
          : `Choose a live agent — ${description}`
      }
      className={cn("relative", className)}
      data-testid="channel-agent-media-trigger"
      // Left off in the several-agents case so the menu trigger can supply its
      // own: Radix merges an `asChild` child's handler with the one it injects,
      // and both would fire.
      onClick={sole ? () => open(sole.agentPubkey) : undefined}
      size="icon"
      type="button"
      variant="outline"
    >
      <Video />
      {/* Distinct from the huddle ring: audio and video are different
          rooms, and two identical pulses beside each other read as one
          feature with a duplicated button. */}
      <span className="absolute inset-0 animate-pulse rounded-lg ring-2 ring-primary/60" />
      {entries.length > 1 ? (
        <span className="absolute -right-1 -top-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-full border border-border bg-background px-0.5 text-2xs font-bold text-muted-foreground">
          {entries.length}
        </span>
      ) : null}
    </Button>
  );

  if (sole) {
    return (
      <Tooltip>
        <TooltipTrigger asChild>{trigger}</TooltipTrigger>
        <TooltipContent>{description}</TooltipContent>
      </Tooltip>
    );
  }

  // No tooltip on this branch: the menu names every agent, and a tooltip
  // wrapping a menu trigger fights the popover for the same pointer.
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        {renderRows()}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
