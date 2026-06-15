import * as React from "react";
import { Activity } from "lucide-react";

import { useUserProfileQuery } from "@/features/profile/hooks";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useProfilePanel } from "@/shared/context/ProfilePanelContext";

import { Popover, PopoverAnchor, PopoverContent } from "@/shared/ui/popover";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { useNow } from "@/shared/lib/useNow";

type UserProfilePopoverProps = {
  children: React.ReactNode;
  pubkey: string;
  triggerElement?: "div" | "span";
  /** When set to "bot", a BotIdenticon badge renders next to the display name. */
  role?: string;
  /** Value used to generate the BotIdenticon glyph (typically the author name). */
  botIdenticonValue?: string;
};

const HOVER_OPEN_DELAY_MS = 300;
const HOVER_CLOSE_DELAY_MS = 200;

const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  aider: "Aider",
};

function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[command] ?? command;
}

function InfoBadge({ children }: { children: React.ReactNode }) {
  return (
    <span className="inline-flex items-center rounded-full bg-muted/50 px-2 py-0.5 text-xs text-muted-foreground">
      {children}
    </span>
  );
}

function truncatePubkey(pubkey: string) {
  if (pubkey.length <= 16) {
    return pubkey;
  }

  return `${pubkey.slice(0, 8)}…${pubkey.slice(-8)}`;
}

export function UserProfilePopover({
  children,
  pubkey,
  triggerElement = "div",
  role,
  botIdenticonValue,
}: UserProfilePopoverProps) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const profileQuery = useUserProfileQuery(open ? pubkey : undefined);
  const relayAgentsQuery = useRelayAgentsQuery({
    enabled: open && role === "bot",
  });
  const managedAgentsQuery = useManagedAgentsQuery({
    enabled: open && role === "bot",
  });
  const presenceQuery = usePresenceQuery(open ? [pubkey] : [], {
    enabled: open,
  });
  const userStatusQuery = useUserStatusQuery(open ? [pubkey] : []);

  const { onOpenAgentSession } = useAgentSession();
  const { openProfilePanel } = useProfilePanel();
  const relayAgent = relayAgentsQuery.data?.find((a) => a.pubkey === pubkey);
  const managedAgent = managedAgentsQuery.data?.find(
    (a) => a.pubkey === pubkey,
  );
  const canViewActivity = role === "bot" && Boolean(onOpenAgentSession);
  const profile = profileQuery.data;
  const presenceStatus = presenceQuery.data?.[pubkey.toLowerCase()];
  const userStatus = userStatusQuery.data?.[pubkey.toLowerCase()];
  const activeTurns = useActiveAgentTurns(role === "bot" ? pubkey : null);
  const channelsQuery = useChannelsQuery();
  const channelIdToName = React.useMemo(() => {
    const map: Record<string, string> = {};
    for (const channel of channelsQuery.data ?? []) {
      map[channel.id] = channel.name;
    }
    return map;
  }, [channelsQuery.data]);

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);

  const handleTriggerMouseEnter = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer]);

  const handleMouseLeave = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);

  const handleContentMouseEnter = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  const handleTriggerClick = React.useCallback(
    (event: React.MouseEvent) => {
      clearHoverTimer();
      if (openProfilePanel) {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        openProfilePanel(pubkey);
      }
    },
    [clearHoverTimer, openProfilePanel, pubkey],
  );

  React.useEffect(() => {
    return () => clearHoverTimer();
  }, [clearHoverTimer]);

  const TriggerElement = triggerElement;

  return (
    <Popover onOpenChange={setOpen} open={open}>
      <PopoverAnchor asChild>
        <TriggerElement
          role="button"
          tabIndex={0}
          onClick={handleTriggerClick}
          onKeyDown={(e) => {
            if ((e.key === "Enter" || e.key === " ") && openProfilePanel) {
              e.preventDefault();
              e.stopPropagation();
              clearHoverTimer();
              setOpen(false);
              openProfilePanel(pubkey);
            }
          }}
          onMouseEnter={handleTriggerMouseEnter}
          onMouseLeave={handleMouseLeave}
          className="inline-flex"
        >
          {children}
        </TriggerElement>
      </PopoverAnchor>
      <PopoverContent
        align="start"
        className="w-80"
        data-testid="user-profile-popover"
        onMouseEnter={handleContentMouseEnter}
        onMouseLeave={handleMouseLeave}
        side="top"
        sideOffset={8}
      >
        <div className="flex flex-col gap-3">
          <div className="flex items-start gap-3">
            {profile?.avatarUrl ? (
              <img
                alt={profile.displayName ?? "User avatar"}
                className="h-10 w-10 shrink-0 rounded-lg object-cover shadow-xs"
                referrerPolicy="no-referrer"
                src={rewriteRelayUrl(profile.avatarUrl)}
              />
            ) : (
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-secondary text-xs font-semibold text-secondary-foreground shadow-xs">
                {(profile?.displayName ?? pubkey.slice(0, 2))
                  .slice(0, 2)
                  .toUpperCase()}
              </div>
            )}

            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <p className="truncate text-sm font-semibold">
                  {profile?.displayName ?? truncatePubkey(pubkey)}
                </p>
                {role === "bot" && botIdenticonValue ? (
                  <BotIdenticon
                    value={botIdenticonValue}
                    size={20}
                    className="shrink-0 rounded"
                  />
                ) : null}
              </div>
              {profile?.nip05Handle ? (
                <p className="truncate text-xs text-muted-foreground">
                  {profile.nip05Handle}
                </p>
              ) : null}
              {profile?.displayName ? (
                <p className="truncate font-mono text-[10px] text-muted-foreground/50">
                  {truncatePubkey(pubkey)}
                </p>
              ) : null}
            </div>

            {presenceStatus ? <PresenceBadge status={presenceStatus} /> : null}
          </div>

          {userStatus ? (
            <p
              className="text-xs text-muted-foreground"
              data-testid="user-profile-status"
            >
              {userStatus.emoji ? (
                <StatusEmoji
                  className="mr-1 h-3.5 w-3.5"
                  value={userStatus.emoji}
                />
              ) : null}
              {userStatus.text}
            </p>
          ) : null}

          {role === "bot" && (managedAgent || relayAgent) ? (
            <div className="flex flex-wrap gap-1.5">
              {managedAgent?.agentCommand ? (
                <InfoBadge>{runtimeLabel(managedAgent.agentCommand)}</InfoBadge>
              ) : relayAgent?.agentType ? (
                <InfoBadge>{runtimeLabel(relayAgent.agentType)}</InfoBadge>
              ) : null}
              {managedAgent?.model ? (
                <InfoBadge>{managedAgent.model}</InfoBadge>
              ) : null}
              {managedAgent?.acpCommand ? (
                <InfoBadge>ACP: {managedAgent.acpCommand}</InfoBadge>
              ) : null}
            </div>
          ) : null}

          {activeTurns.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {activeTurns.map(({ channelId, observedAt }) => (
                <PopoverWorkingBadge
                  key={channelId}
                  name={channelIdToName[channelId] ?? channelId}
                  observedAt={observedAt}
                />
              ))}
            </div>
          ) : null}

          {profile?.about ? (
            <p className="text-xs leading-relaxed text-muted-foreground">
              {profile.about}
            </p>
          ) : null}

          {canViewActivity ? (
            <button
              className="flex w-full items-center gap-2 rounded-lg border border-border/60 px-3 py-2 text-left text-xs font-medium text-foreground transition-colors hover:bg-muted/50"
              data-testid={`user-profile-view-activity-${pubkey}`}
              onClick={() => {
                setOpen(false);
                onOpenAgentSession?.(pubkey);
              }}
              type="button"
            >
              <Activity className="h-4 w-4 text-muted-foreground" />
              View activity log
            </button>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}

function PopoverWorkingBadge({
  name,
  observedAt,
}: {
  name: string;
  observedAt: number;
}) {
  const now = useNow(1000);

  return (
    <span className="inline-flex items-center rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary motion-safe:animate-pulse">
      Working in #{name} · {formatElapsed(now - observedAt)}
    </span>
  );
}
