import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  ArrowUpRight,
  Brain,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Copy,
  Cpu,
  Fingerprint,
  Hash,
  MessageSquare,
  Pencil,
  Server,
  Terminal,
  UserMinus,
  UserPlus,
  UserRound,
} from "lucide-react";
import { toast } from "sonner";

import { MemorySection } from "@/features/agent-memory/ui/MemorySection";
import { AgentStatusBadge } from "@/features/agents/ui/AgentStatusBadge";
import { useActiveAgentTurns } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { getPresenceLabel } from "@/features/presence/lib/presence";
import { PresenceDot } from "@/features/presence/ui/PresenceBadge";
import type {
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import { truncatePubkey as truncatePubkeyShort } from "@/features/profile/lib/identity";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { useFeatureEnabled } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";

const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  aider: "Aider",
};

function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[command] ?? command;
}

async function copyToClipboard(value: string, label?: string) {
  await navigator.clipboard.writeText(value);
  toast.success(label ? `Copied ${label}` : "Copied to clipboard");
}

// ── Summary view ─────────────────────────────────────────────────────────────

export type ProfileSummaryViewProps = {
  canEditAgent: boolean;
  canViewActivity: boolean;
  channelCount: number;
  channelIdToName: Record<string, string>;
  channelsLoading: boolean;
  displayName: string;
  followMutation: ReturnType<typeof useFollowMutation>;
  handleEditAgent: () => void;
  handleMessage: () => void;
  handleOpenActivity: () => void;
  isBot: boolean;
  isFollowing: boolean;
  isOwner: boolean | undefined;
  isSelf: boolean;
  managedAgent: ManagedAgent | undefined;
  memoriesLoading: boolean;
  memoryCount: number | undefined;
  ownerDisplayName: string | null;
  ownerHandle: string | null;
  onOpenChannels: () => void;
  onOpenMemories: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  presenceLoaded: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ReturnType<typeof useUserProfileQuery>["data"];
  pubkey: string;
  relayAgent: RelayAgent | undefined;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  userStatus: { text: string; emoji: string } | null | undefined;
};

export function ProfileSummaryView({
  canEditAgent,
  canViewActivity,
  channelCount,
  channelIdToName,
  channelsLoading,
  displayName,
  followMutation,
  handleEditAgent,
  handleMessage,
  handleOpenActivity,
  isBot,
  isFollowing,
  isOwner,
  isSelf,
  managedAgent,
  memoriesLoading,
  memoryCount,
  ownerDisplayName,
  ownerHandle,
  onOpenChannels,
  onOpenMemories,
  onOpenDm,
  presenceLoaded,
  presenceStatus,
  profile,
  pubkey,
  relayAgent,
  unfollowMutation,
  userStatus,
}: ProfileSummaryViewProps) {
  const { goChannel } = useAppNavigation();
  const activeTurns = useActiveAgentTurns(isBot ? pubkey : null);

  const metadataFields = [
    ...buildPublicFields({
      pubkey,
      profile,
      relayAgent,
      isBot,
    }),
    ...(isOwner === true
      ? buildOwnerFields({
          managedAgent,
          ownerDisplayName,
          ownerHandle,
          presenceLoaded,
          presenceStatus,
          relayAgent,
        })
      : []),
  ];

  const showMemoriesIngress = isOwner === true;
  const showChannelsIngress =
    channelsLoading || channelCount > 0 || isBot || relayAgent !== undefined;

  return (
    <div className="flex flex-col gap-6 pt-4">
      <ProfileHero
        displayName={displayName}
        isBot={isBot}
        presenceStatus={presenceStatus}
        profile={profile}
        userStatus={userStatus}
      />

      {!isSelf ? (
        <ProfilePrimaryActions
          canEditAgent={canEditAgent}
          followMutation={followMutation}
          onEditAgent={handleEditAgent}
          isFollowing={isFollowing}
          onMessage={onOpenDm ? handleMessage : undefined}
          pubkey={pubkey}
          unfollowMutation={unfollowMutation}
        />
      ) : null}

      {activeTurns.length > 0 ? (
        <div className="flex flex-wrap justify-center gap-1.5">
          {activeTurns.map(({ channelId, anchorAt }) => (
            <ProfileWorkingBadge
              key={channelId}
              channelId={channelId}
              name={channelIdToName[channelId] ?? channelId}
              anchorAt={anchorAt}
              onNavigate={goChannel}
            />
          ))}
        </div>
      ) : null}

      {showMemoriesIngress || showChannelsIngress || canViewActivity ? (
        <section className="space-y-2">
          {showMemoriesIngress ? (
            <ProfileIngressRow
              icon={Brain}
              label="Memories"
              onClick={onOpenMemories}
              testId="user-profile-memories-ingress"
              trailing={
                memoriesLoading
                  ? "Loading…"
                  : memoryCount !== undefined
                    ? String(memoryCount)
                    : "View"
              }
            />
          ) : null}
          {showChannelsIngress ? (
            <ProfileIngressRow
              icon={Hash}
              label="Channels"
              onClick={onOpenChannels}
              testId="user-profile-channels-ingress"
              trailing={
                channelsLoading
                  ? "Loading…"
                  : channelCount > 0
                    ? String(channelCount)
                    : "None"
              }
            />
          ) : null}
          {canViewActivity ? (
            <ProfileIngressRow
              icon={Activity}
              label="Activity log"
              onClick={handleOpenActivity}
              testId={`user-profile-view-activity-${pubkey}`}
            />
          ) : null}
        </section>
      ) : null}

      {metadataFields.length > 0 ? (
        <ProfileFieldGroup fields={metadataFields} />
      ) : null}
    </div>
  );
}

function ProfileWorkingBadge({
  channelId,
  name,
  anchorAt,
  onNavigate,
}: {
  channelId: string;
  name: string;
  anchorAt: number;
  onNavigate: (channelId: string) => void;
}) {
  const now = useNow(1000);

  return (
    <Badge
      className="cursor-pointer motion-safe:animate-pulse normal-case tracking-normal hover:opacity-80"
      variant="default"
      onClick={() => onNavigate(channelId)}
    >
      Working in #{name} · {formatElapsed(now - anchorAt)}
    </Badge>
  );
}

// ── Hero & metadata ──────────────────────────────────────────────────────────

function ProfileHero({
  displayName,
  isBot,
  presenceStatus,
  profile,
  userStatus,
}: {
  displayName: string;
  isBot: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  profile: ProfileSummaryViewProps["profile"];
  userStatus: ProfileSummaryViewProps["userStatus"];
}) {
  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <div className="relative">
        <ProfileAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          className="h-20 w-20 text-xl"
          iconClassName="h-8 w-8"
          label={displayName}
          plain
          testId="user-profile-avatar"
        />
        {presenceStatus ? (
          <span
            aria-label={getPresenceLabel(presenceStatus)}
            className="absolute bottom-0 right-0 flex h-6 w-6 items-center justify-center rounded-full bg-background"
            data-testid="user-profile-presence-badge"
            role="img"
          >
            <PresenceDot className="h-3.5 w-3.5" status={presenceStatus} />
          </span>
        ) : null}
      </div>

      <div className="flex flex-col items-center gap-1">
        <div className="flex items-center justify-center gap-2">
          <h3 className="text-xl font-semibold tracking-tight">
            {displayName}
          </h3>
          {isBot ? (
            <BotIdenticon
              className="shrink-0 rounded"
              size={20}
              value={displayName}
            />
          ) : null}
        </div>

        {profile?.about?.trim() ? (
          <ProfileHeroDescription
            about={profile.about.trim()}
            key={profile.about.trim()}
          />
        ) : null}

        {profile?.nip05Handle ? (
          <p className="text-sm text-muted-foreground">{profile.nip05Handle}</p>
        ) : null}

        {userStatus ? (
          <p className="text-sm text-muted-foreground">
            {userStatus.emoji ? (
              <StatusEmoji
                className="mr-1 inline h-3.5 w-3.5"
                value={userStatus.emoji}
              />
            ) : null}
            {userStatus.text}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function ProfileHeroDescription({ about }: { about: string }) {
  const [expanded, setExpanded] = React.useState(false);
  const [isTruncated, setIsTruncated] = React.useState(false);
  const textRef = React.useRef<HTMLParagraphElement>(null);

  const measureTruncation = React.useCallback(() => {
    const element = textRef.current;
    if (!element || expanded) {
      return;
    }
    setIsTruncated(element.scrollHeight > element.clientHeight + 1);
  }, [expanded]);

  React.useLayoutEffect(() => {
    measureTruncation();
  }, [measureTruncation]);

  React.useEffect(() => {
    const element = textRef.current;
    if (!element) {
      return;
    }

    const observer = new ResizeObserver(() => {
      measureTruncation();
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [measureTruncation]);

  const toggleClassName =
    "inline-flex items-center gap-0.5 text-xs font-medium text-muted-foreground opacity-60 transition-opacity hover:text-foreground hover:opacity-100";

  return (
    <div className="flex w-full flex-col items-center gap-0.5">
      <div className="w-fit max-w-full px-2">
        <p
          className={cn(
            "text-center whitespace-pre-wrap text-sm leading-relaxed text-muted-foreground",
            !expanded && "line-clamp-3",
          )}
          data-testid="user-profile-description"
          ref={textRef}
        >
          {about}
        </p>
      </div>
      {!expanded && isTruncated ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(true)}
          type="button"
        >
          more
          <ChevronDown className="h-4 w-4" />
        </button>
      ) : null}
      {expanded ? (
        <button
          className={toggleClassName}
          data-testid="user-profile-description-toggle"
          onClick={() => setExpanded(false)}
          type="button"
        >
          less
          <ChevronUp className="h-4 w-4" />
        </button>
      ) : null}
    </div>
  );
}

// ── Primary actions ──────────────────────────────────────────────────────────

function ProfilePrimaryActions({
  canEditAgent,
  followMutation,
  isFollowing,
  onEditAgent,
  onMessage,
  pubkey,
  unfollowMutation,
}: {
  canEditAgent: boolean;
  followMutation: ReturnType<typeof useFollowMutation>;
  isFollowing: boolean;
  onEditAgent: () => void;
  onMessage?: () => void;
  pubkey: string;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
}) {
  const showFollowAction = useFeatureEnabled("pulse");
  const followToggleMutation = isFollowing ? unfollowMutation : followMutation;

  const handleFollowClick = () => {
    followToggleMutation.mutate(pubkey, {
      onError: (error) =>
        toast.error(
          `${isFollowing ? "Unfollow" : "Follow"} failed: ${error.message}`,
        ),
    });
  };

  return (
    <div className="flex items-start justify-center gap-8">
      {showFollowAction ? (
        <ProfileQuickAction
          active={isFollowing}
          disabled={followToggleMutation.isPending}
          icon={isFollowing ? UserMinus : UserPlus}
          label={isFollowing ? "Unfollow" : "Follow"}
          onClick={handleFollowClick}
        />
      ) : null}
      {onMessage ? (
        <ProfileQuickAction
          icon={MessageSquare}
          label="Message"
          onClick={onMessage}
          testId="user-profile-message"
        />
      ) : null}
      {canEditAgent ? (
        <ProfileQuickAction
          icon={Pencil}
          label="Edit"
          onClick={onEditAgent}
          testId="user-profile-edit-agent"
        />
      ) : null}
    </div>
  );
}

function ProfileQuickAction({
  active,
  disabled,
  icon: Icon,
  label,
  onClick,
  testId,
}: {
  active?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <button
      className="flex flex-col items-center gap-2 disabled:cursor-not-allowed disabled:opacity-50"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span
        className={cn(
          "flex h-14 w-14 items-center justify-center rounded-full transition-colors",
          active
            ? "bg-foreground text-background hover:bg-foreground/90"
            : "bg-muted/60 text-foreground hover:bg-muted/80",
        )}
      >
        <Icon className="h-4 w-4" />
      </span>
      <span
        className={cn(
          "text-xs",
          active ? "text-foreground" : "text-muted-foreground",
        )}
      >
        {label}
      </span>
    </button>
  );
}

// ── Field rows ───────────────────────────────────────────────────────────────

type ProfileField = {
  copyValue?: string;
  /**
   * Plain-text representation. Always required so non-visual surfaces (e.g. tooltips,
   * copy-to-clipboard) keep working. When `displayNode` is set, the row renders that
   * instead of the text — but the text still drives the title/tooltip.
   */
  displayValue: string;
  /**
   * Optional rich rendering for the value cell (e.g. a status badge). When present,
   * replaces the plain text node in the row.
   */
  displayNode?: React.ReactNode;
  icon: LucideIcon;
  label: string;
  testId?: string;
};

function buildPublicFields({
  isBot,
  profile,
  pubkey,
  relayAgent,
}: {
  isBot: boolean;
  profile: ProfileSummaryViewProps["profile"];
  pubkey: string;
  relayAgent: RelayAgent | undefined;
}): ProfileField[] {
  const fields: ProfileField[] = [
    {
      copyValue: pubkey,
      displayValue: truncatePubkeyShort(pubkey),
      icon: Fingerprint,
      label: "Public key",
      testId: "user-profile-copy-pubkey",
    },
  ];

  if (profile?.nip05Handle) {
    fields.push({
      copyValue: profile.nip05Handle,
      displayValue: profile.nip05Handle,
      icon: UserRound,
      label: "NIP-05",
      testId: "user-profile-nip05",
    });
  }

  if (isBot && relayAgent?.agentType) {
    fields.push({
      copyValue: relayAgent.agentType,
      displayValue: runtimeLabel(relayAgent.agentType),
      icon: Cpu,
      label: "Agent type",
      testId: "user-profile-agent-type",
    });
  }

  if (relayAgent?.capabilities.length) {
    fields.push({
      copyValue: relayAgent.capabilities.join(", "),
      displayValue: relayAgent.capabilities.join(", "),
      icon: Server,
      label: "Capabilities",
      testId: "user-profile-capabilities",
    });
  }

  return fields;
}

function buildOwnerFields({
  managedAgent,
  ownerDisplayName,
  ownerHandle,
  presenceLoaded,
  presenceStatus,
  relayAgent,
}: {
  managedAgent: ManagedAgent | undefined;
  ownerDisplayName: string | null;
  ownerHandle: string | null;
  presenceLoaded: boolean;
  presenceStatus: "online" | "away" | "offline" | undefined;
  relayAgent: RelayAgent | undefined;
}): ProfileField[] {
  const fields: ProfileField[] = [];

  if (ownerDisplayName) {
    fields.push({
      copyValue: ownerHandle ?? undefined,
      displayValue: ownerDisplayName,
      icon: UserRound,
      label: "Owned by",
      testId: "user-profile-owned-by",
    });
  }

  if (managedAgent?.agentCommand) {
    fields.push({
      copyValue: managedAgent.agentCommand,
      displayValue: runtimeLabel(managedAgent.agentCommand),
      icon: Terminal,
      label: "Runtime",
      testId: "user-profile-runtime",
    });
  } else if (relayAgent?.agentType) {
    fields.push({
      copyValue: relayAgent.agentType,
      displayValue: runtimeLabel(relayAgent.agentType),
      icon: Terminal,
      label: "Runtime",
      testId: "user-profile-runtime",
    });
  }

  if (managedAgent) {
    fields.push({
      displayValue: managedAgent.status
        .replace(/_/g, " ")
        .replace(/\b\w/g, (char: string) => char.toUpperCase()),
      displayNode: (
        <AgentStatusBadge
          presenceLoaded={presenceLoaded}
          presenceStatus={presenceStatus}
          status={managedAgent.status}
        />
      ),
      icon: Activity,
      label: "Status",
      testId: "user-profile-agent-status",
    });
  }

  if (managedAgent?.model) {
    fields.push({
      copyValue: managedAgent.model,
      displayValue: managedAgent.model,
      icon: Cpu,
      label: "Model",
      testId: "user-profile-model",
    });
  }

  if (managedAgent?.acpCommand) {
    fields.push({
      copyValue: managedAgent.acpCommand,
      displayValue: managedAgent.acpCommand,
      icon: Terminal,
      label: "ACP command",
      testId: "user-profile-acp",
    });
  }

  if (managedAgent?.mcpCommand) {
    fields.push({
      copyValue: managedAgent.mcpCommand,
      displayValue: managedAgent.mcpCommand,
      icon: Terminal,
      label: "MCP command",
      testId: "user-profile-mcp",
    });
  }

  if (managedAgent?.backend.type === "provider") {
    const backendLabel = managedAgent.backend.id;
    fields.push({
      copyValue: backendLabel,
      displayValue: backendLabel,
      icon: Server,
      label: "Backend",
      testId: "user-profile-backend",
    });
  }

  if (managedAgent) {
    fields.push({
      displayValue: managedAgent.startOnAppLaunch ? "Yes" : "No",
      icon: Server,
      label: "Start on launch",
      testId: "user-profile-start-on-launch",
    });
    fields.push({
      displayValue: managedAgent.respondTo.replace(/-/g, " "),
      icon: MessageSquare,
      label: "Respond to",
      testId: "user-profile-respond-to",
    });
  }

  if (managedAgent?.lastError) {
    fields.push({
      copyValue: managedAgent.lastError,
      displayValue: managedAgent.lastError,
      icon: Activity,
      label: "Last error",
      testId: "user-profile-last-error",
    });
  }

  return fields;
}

function ProfileFieldGroup({ fields }: { fields: ProfileField[] }) {
  const publicKeyLabel = "Public key";
  const ownedByLabel = "Owned by";
  const statusLabel = "Status";
  const orderedFields = [
    ...fields.filter((field) => field.label === publicKeyLabel),
    ...fields.filter((field) => field.label === ownedByLabel),
    ...fields.filter(
      (field) =>
        field.label !== publicKeyLabel &&
        field.label !== ownedByLabel &&
        field.copyValue,
    ),
    ...fields.filter((field) => field.label === statusLabel),
    ...fields.filter((field) => {
      if (
        field.label === publicKeyLabel ||
        field.label === ownedByLabel ||
        field.label === statusLabel
      ) {
        return false;
      }
      return !field.copyValue;
    }),
  ];

  return (
    <section>
      <div className="overflow-hidden rounded-2xl bg-muted/20">
        {orderedFields.map((field) => (
          <ProfileFieldRow field={field} key={field.testId ?? field.label} />
        ))}
      </div>
    </section>
  );
}

function ProfileFieldRow({ field }: { field: ProfileField }) {
  const Icon = field.icon;
  const isCopyable = Boolean(field.copyValue);

  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-xs font-medium text-foreground">
          {field.label}
        </span>
        <span
          className="mt-0.5 block truncate text-sm text-muted-foreground"
          title={field.displayValue}
        >
          {field.displayNode ?? field.displayValue}
        </span>
      </span>
      {isCopyable ? (
        <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </>
  );

  if (isCopyable && field.copyValue) {
    return (
      <button
        aria-label={`Copy ${field.label}`}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        data-testid={field.testId}
        onClick={() => void copyToClipboard(field.copyValue ?? "", field.label)}
        title={`Copy ${field.label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className="flex items-center gap-3 px-4 py-3"
      data-testid={field.testId}
    >
      {content}
    </div>
  );
}

// ── Ingress rows ─────────────────────────────────────────────────────────────

function ProfileIngressRow({
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: string;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-2 text-left transition-colors hover:bg-muted/40"
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-sm font-medium text-foreground">
        {label}
      </span>
      {trailing ? (
        <span className="text-sm text-muted-foreground">{trailing}</span>
      ) : null}
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

// ── Focused views ────────────────────────────────────────────────────────────

export function MemoryFocusedView({
  agentPubkey,
  isOwner,
}: {
  agentPubkey: string;
  isOwner: boolean | undefined;
}) {
  if (isOwner !== true) {
    return null;
  }

  return (
    <div className="pt-4">
      <MemorySection agentPubkey={agentPubkey} />
    </div>
  );
}

type ProfileChannelLink = {
  id: string;
  name: string;
};

export function ChannelsFocusedView({
  channels,
  isLoading,
  onOpenChannel,
}: {
  channels: ProfileChannelLink[];
  isLoading: boolean;
  onOpenChannel: (channelId: string) => void;
}) {
  if (isLoading) {
    return (
      <p className="pt-4 text-base leading-7 text-muted-foreground">
        Loading channels…
      </p>
    );
  }

  if (channels.length === 0) {
    return (
      <p
        className="pt-4 text-base leading-7 italic text-muted-foreground"
        data-testid="user-profile-channels-empty"
      >
        No visible channel memberships.
      </p>
    );
  }

  return (
    <ul
      className="overflow-hidden rounded-2xl bg-muted/20"
      data-testid="user-profile-channels-list"
    >
      {channels.map((channel) => (
        <li key={channel.id}>
          <button
            aria-label={`Open #${channel.name}`}
            className="group flex w-full items-center gap-3 px-4 py-3 text-left text-base leading-7 text-foreground transition-colors hover:bg-muted/40"
            data-testid={`user-profile-channel-link-${channel.name}`}
            onClick={() => onOpenChannel(channel.id)}
            type="button"
          >
            <span className="min-w-0 flex-1 truncate">#{channel.name}</span>
            <ArrowUpRight
              aria-hidden="true"
              className="h-4 w-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground"
            />
          </button>
        </li>
      ))}
    </ul>
  );
}
