import { ArrowUpRight, MessageSquare } from "lucide-react";

import { UsageAndActivityButton } from "@/features/agents/ui/AgentUsageActivityPanel";

import { AgentIdentityCard } from "@/features/agents/ui/AgentIdentityCard";
import { IdentityInitialsAvatar } from "@/features/agents/ui/IdentityInitialsAvatar";
import { IDENTITY_CARD_GRID_CLASS } from "@/features/agents/ui/UnifiedAgentsSection";
import {
  communityBotDirectoryChannelLink,
  type CommunityBotDirectoryCard,
  type CommunityBotDirectoryDetail,
} from "@/features/community-bots/lib/directory";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { ProfileSectionGroup } from "@/features/profile/ui/UserProfilePanelFields";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { PubKey } from "@/shared/ui/PubKey";
import { Spinner } from "@/shared/ui/spinner";

export function BotsEmptyState({
  onOpenSettings,
}: {
  onOpenSettings: () => void;
}) {
  return (
    <div
      className="rounded-2xl border border-dashed border-border/80 px-6 py-12 text-center"
      data-testid="bots-empty-state"
    >
      <p className="text-base font-medium text-foreground">
        No community bots yet
      </p>
      <p className="mt-2 text-sm text-muted-foreground">
        Install them in Settings → Communities → Bots.
      </p>
      <Button
        className="mt-5"
        data-testid="bots-empty-open-settings"
        onClick={onOpenSettings}
        size="sm"
        type="button"
        variant="outline"
      >
        Open Bots settings
      </Button>
    </div>
  );
}

export function BotsDirectoryGrid({
  cards,
  onOpenBot,
}: {
  cards: readonly CommunityBotDirectoryCard[];
  onOpenBot: (botId: string) => void;
}) {
  return (
    <div className={IDENTITY_CARD_GRID_CLASS} data-testid="bots-directory">
      {cards.map((card) => (
        <AgentIdentityCard
          ariaLabel={`Open ${card.name}`}
          avatarUrl={card.avatarUrl}
          dataTestId={`bot-card-${card.id}`}
          key={card.id}
          label={card.name}
          subtitle={card.status.label}
          onClick={() => onOpenBot(card.id)}
        />
      ))}
    </div>
  );
}

export function BotDetailContent({
  detail,
  messagePending = false,
  onMessage,
  onOpenChannel,
}: {
  detail: CommunityBotDirectoryDetail;
  messagePending?: boolean;
  onMessage?: () => void;
  onOpenChannel: (channelId: string) => void;
}) {
  const channelLinks = detail.channels.map(communityBotDirectoryChannelLink);

  return (
    <div className="flex flex-col gap-6 pt-8" data-testid="bot-detail-content">
      <div className="flex flex-col items-center gap-3 text-center">
        {/*
          Sized outer span + extra top padding keep the hero avatar fully
          visible under the overlapping Profile header (overflow clips shadows
          / borders when the avatar sits flush with the scroll edge).
        */}
        <span
          className="relative inline-flex h-20 w-20 shrink-0"
          data-testid="bot-detail-avatar"
        >
          {detail.avatarUrl ? (
            <ProfileAvatar
              avatarUrl={detail.avatarUrl}
              className="h-full w-full border-[3px] border-background bg-muted shadow-none"
              iconClassName="h-8 w-8"
              label={detail.name}
              openClawBadgeAvatarPx={80}
            />
          ) : (
            <IdentityInitialsAvatar
              className="shadow-none"
              label={detail.name}
              size={80}
            />
          )}
        </span>
        <div className="flex max-w-full flex-col items-center gap-1">
          <h2
            className="truncate text-xl font-semibold tracking-tight"
            data-testid="bot-detail-name"
          >
            {detail.name}
          </h2>
          <Badge data-testid="bot-detail-status" variant="secondary">
            {detail.status.label}
          </Badge>
          {detail.description ? (
            <p
              className="max-w-full px-2 text-center whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground"
              data-testid="bot-detail-description"
            >
              {detail.description}
            </p>
          ) : null}
        </div>
      </div>

      {onMessage ? (
        <div
          className="grid grid-flow-col auto-cols-fr gap-2"
          data-testid="bot-detail-primary-actions"
        >
          <button
            aria-busy={messagePending || undefined}
            aria-label="Message"
            className="flex min-h-20 w-full flex-col items-center justify-center gap-1.5 rounded-xl bg-muted px-2 py-3 text-center transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
            data-testid="bot-detail-message"
            disabled={messagePending}
            onClick={onMessage}
            type="button"
          >
            {messagePending ? (
              <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
            ) : (
              <MessageSquare className="h-5 w-5 text-foreground" />
            )}
            <span className="min-w-0 text-xs font-medium leading-tight">
              Message
            </span>
          </button>
        </div>
      ) : null}

      <UsageAndActivityButton
        agentName={detail.name}
        agentPubkey={detail.hexPubkey}
        className="w-full justify-start gap-2 border border-border/60 px-3 py-2 text-xs font-medium"
        label="Usage & activity"
        size="sm"
        testId={`bot-detail-usage-activity-${detail.hexPubkey}`}
        variant="ghost"
      />

      <ProfileSectionGroup testId="bot-detail-public-key" title="Identity">
        <div className="flex min-h-16 items-center justify-between gap-4 px-4 py-3">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">Public key</p>
            <PubKey pubkey={detail.hexPubkey} testId="bot-detail-pubkey" />
          </div>
        </div>
      </ProfileSectionGroup>

      <ProfileSectionGroup testId="bot-detail-channels" title="Channels">
        {channelLinks.length === 0 ? (
          <p
            className="px-4 py-3 text-sm leading-6 text-muted-foreground"
            data-testid="bot-detail-channels-empty"
          >
            This bot is not a member of any channels.
          </p>
        ) : (
          <ul
            className="divide-y divide-border/55"
            data-testid="bot-detail-channels-list"
          >
            {channelLinks.map((channel) => (
              <li key={channel.id}>
                <button
                  aria-label={channel.ariaLabel}
                  className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left text-sm font-medium text-foreground transition-colors hover:bg-muted/40"
                  data-testid={`bot-detail-channel-${channel.id}`}
                  onClick={() => onOpenChannel(channel.id)}
                  type="button"
                >
                  <span className="min-w-0 flex-1 truncate">
                    {channel.label}
                  </span>
                  <ArrowUpRight
                    aria-hidden="true"
                    className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
                  />
                </button>
              </li>
            ))}
          </ul>
        )}
      </ProfileSectionGroup>
    </div>
  );
}
