import { Activity, Circle } from "lucide-react";

import {
  describeSharedAgentActivity,
  type AgentActivityMode,
} from "@/features/agents/sharedAgentActivity";
import { useSharedAgentActivity } from "@/features/agents/useSharedAgentActivity";
import { formatDurationMs } from "@/features/agents/ui/agentSessionUtils";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
} from "@/shared/layout/AuxiliaryPanel";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Spinner } from "@/shared/ui/spinner";

export function SharedAgentActivityPanel({
  agent,
  channel,
  isSinglePanelView = false,
  layout = "standalone",
  mode,
  onBack,
  onClose,
  profiles,
  transparentChrome = false,
  widthPx,
}: {
  agent: { pubkey: string; name: string };
  channel: Channel | null;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  mode: Exclude<AgentActivityMode, "owner">;
  onBack?: () => void;
  onClose: () => void;
  profiles?: UserProfileLookup;
  transparentChrome?: boolean;
  widthPx: number;
}) {
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay || isSinglePanelView);
  const profile = profiles?.[normalizePubkey(agent.pubkey)] ?? null;
  const label = resolveUserLabel({
    pubkey: agent.pubkey,
    fallbackName: agent.name,
    profiles,
    preferResolvedSelfLabel: true,
  });
  const { activities, connection } = useSharedAgentActivity({
    enabled: mode === "shared",
    agentPubkey: agent.pubkey,
    channelId: channel?.id ?? null,
  });
  const scope = channel
    ? `Shared activity · #${channel.name}`
    : "Shared activity";

  return (
    <AuxiliaryPanel
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="agent-session-thread-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={layout !== "split" && !isOverlay}
          backdropSurface="soft"
          inset={layout !== "split" ? "wide" : "default"}
        >
          <AuxiliaryPanelHeaderGroup
            align="start"
            backButtonAriaLabel="Back from activity"
            backButtonTestId="agent-session-back"
            onBack={onBack}
          >
            <ProfileAvatar
              avatarUrl={profile?.avatarUrl ?? null}
              className="size-9"
              label={label}
              testId="agent-session-agent-avatar"
            />
            <div className="min-w-0 flex-1">
              <h2
                className="truncate text-sm font-semibold leading-5"
                data-testid="agent-session-agent-name"
                title={label}
              >
                {label}
              </h2>
              <p
                className="truncate text-xs text-muted-foreground"
                data-testid="agent-session-scope-label"
              >
                {scope}
              </p>
            </div>
          </AuxiliaryPanelHeaderGroup>
          <AuxiliaryPanelHeaderActions>
            {mode === "shared" ? (
              <span
                className="flex items-center gap-1.5 rounded-full bg-primary/10 px-2 py-1 text-xs text-primary"
                data-testid="shared-agent-activity-live-badge"
              >
                <Circle className="size-2 fill-current" />
                Live
              </span>
            ) : null}
          </AuxiliaryPanelHeaderActions>
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody className="overflow-y-auto px-4 pb-6" panelPadding>
        {mode === "unavailable" ? (
          <EmptyState
            description="Activity is available to members in the agent's shared channels."
            title="Activity unavailable"
          />
        ) : connection === "closed" ? (
          <EmptyState
            description="Your access changed. Reopen the panel after channel membership is restored."
            title="Activity access ended"
          />
        ) : connection === "error" ? (
          <EmptyState
            description="Close and reopen the panel to reconnect."
            title="Activity connection unavailable"
          />
        ) : activities.length === 0 ? (
          <div className="flex min-h-80 flex-col items-center justify-center gap-4 text-center text-muted-foreground">
            {connection === "connecting" ? (
              <Spinner size={32} />
            ) : (
              <Activity className="size-8" />
            )}
            <div>
              <p className="text-sm font-medium text-foreground">
                Waiting for activity…
              </p>
              <p className="mt-1 text-xs">
                New activity appears here live. Earlier activity is not loaded.
              </p>
            </div>
          </div>
        ) : (
          <ol
            className="space-y-2 py-3"
            data-testid="shared-agent-activity-list"
          >
            {activities.map((item) => {
              const description = describeSharedAgentActivity(item);
              const duration =
                item.durationMs == null
                  ? null
                  : formatDurationMs(item.durationMs);
              return (
                <li
                  className="rounded-lg border border-border/60 bg-card/40 px-3 py-2.5"
                  key={item.activityId}
                >
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="truncate text-sm font-medium">
                        {description.label}
                      </p>
                      <p className="mt-0.5 text-xs text-muted-foreground">
                        {description.detail}
                      </p>
                    </div>
                    {duration ? (
                      <span className="shrink-0 text-xs text-muted-foreground">
                        {duration}
                      </span>
                    ) : null}
                  </div>
                </li>
              );
            })}
          </ol>
        )}
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}

function EmptyState({
  description,
  title,
}: {
  description: string;
  title: string;
}) {
  return (
    <div className="flex min-h-80 flex-col items-center justify-center gap-3 px-5 text-center">
      <Activity className="size-8 text-muted-foreground" />
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="mt-1 text-xs text-muted-foreground">{description}</p>
      </div>
    </div>
  );
}
