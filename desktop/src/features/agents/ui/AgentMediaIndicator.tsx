import { Video } from "lucide-react";
import * as React from "react";

import { useAgentMediaSessions } from "@/features/agents/lib/useAgentMediaSessions";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { DropdownMenuItem } from "@/shared/ui/dropdown-menu";
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
 * Clicking opens the agent's session panel, where `AgentMediaSurface` mounts
 * and requests the viewer token. `useOpenAgentActivity` covers both routes:
 * in a channel view an `AgentSessionProvider` opens the panel in place;
 * elsewhere (e.g. the inbox) it navigates to the channel first.
 */
export function AgentMediaIndicator({
  channelId,
  className,
  renderMode = "button",
}: AgentMediaIndicatorProps) {
  const sessions = useAgentMediaSessions(channelId);
  const { openAgentActivity } = useOpenAgentActivity();
  const agentPubkeys = React.useMemo(
    () => sessions.map((session) => session.agentPubkey),
    [sessions],
  );
  // Named agents read far better than truncated keys, and the batch query is
  // disabled on an empty list — so a channel with no session costs no request.
  const profilesQuery = useUsersBatchQuery(agentPubkeys);
  const profiles = profilesQuery.data?.profiles;

  const labels = React.useMemo(
    () =>
      sessions.map((session) =>
        resolveUserLabel({ pubkey: session.agentPubkey, profiles }),
      ),
    [profiles, sessions],
  );

  if (sessions.length === 0) {
    return null;
  }

  // Newest first (the hook's order): the session a member most likely means.
  const primary = sessions[0];
  const sessionCount = sessions.length;
  const description =
    sessionCount === 1
      ? `${labels[0]} is live`
      : `${sessionCount} agents are live`;

  function handleOpen() {
    openAgentActivity(primary.agentPubkey, { channelId });
  }

  if (renderMode === "menu-item") {
    return (
      <DropdownMenuItem
        className={className}
        data-testid="channel-agent-media-trigger"
        onSelect={handleOpen}
      >
        <Video />
        <span>{description}</span>
        {sessionCount > 1 ? (
          <span className="ml-auto text-xs text-muted-foreground">
            {sessionCount}
          </span>
        ) : null}
      </DropdownMenuItem>
    );
  }

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={`Open live video — ${description}`}
          className={cn("relative", className)}
          data-testid="channel-agent-media-trigger"
          onClick={handleOpen}
          size="icon"
          type="button"
          variant="outline"
        >
          <Video />
          {/* Distinct from the huddle ring: audio and video are different
              rooms, and two identical pulses beside each other read as one
              feature with a duplicated button. */}
          <span className="absolute inset-0 animate-pulse rounded-lg ring-2 ring-primary/60" />
          {sessionCount > 1 ? (
            <span className="absolute -right-1 -top-1 flex h-3.5 min-w-3.5 items-center justify-center rounded-full border border-border bg-background px-0.5 text-2xs font-bold text-muted-foreground">
              {sessionCount}
            </span>
          ) : null}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{description}</TooltipContent>
    </Tooltip>
  );
}
