import * as React from "react";
import {
  Activity,
  Archive,
  ArchiveRestore,
  Copy,
  MessageSquare,
  X,
} from "lucide-react";
import { toast } from "sonner";

import {
  useContactListQuery,
  useFollowMutation,
  useUnfollowMutation,
  useUserProfileQuery,
} from "@/features/profile/hooks";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import {
  useArchiveIdentityMutation,
  useIsIdentityArchived,
  useOaOwnerQuery,
  useUnarchiveIdentityMutation,
} from "@/features/identity-archive/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useMyRelayMembershipQuery } from "@/features/relay-members/hooks";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

type UserProfilePanelProps = {
  canResetWidth: boolean;
  currentPubkey?: string;
  isSinglePanelView?: boolean;
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  pubkey: string;
  /**
   * When true, the panel sits beside a sibling pane managed by a single-panel
   * width controller (ChannelScreen). The width is clamped so the sibling keeps
   * at least THREAD_PANEL_MIN_WIDTH_PX. Standalone/floating mounts (e.g. Pulse)
   * have no such sibling, so they omit this and use the configured width
   * directly — otherwise `calc(100% - 300px)` would wrongly shrink the panel.
   */
  splitPaneClamp?: boolean;
  widthPx: number;
};

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

export function UserProfilePanel({
  canResetWidth,
  currentPubkey,
  isSinglePanelView = false,
  onClose,
  onOpenDm,
  onResetWidth,
  onResizeStart,
  pubkey,
  splitPaneClamp = false,
  widthPx,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const usesChannelSplitChrome =
    splitPaneClamp && !isOverlay && !isSinglePanelView;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const profileQuery = useUserProfileQuery(pubkey);
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const presenceQuery = usePresenceQuery([pubkey]);
  const userStatusQuery = useUserStatusQuery([pubkey]);
  const myMembershipQuery = useMyRelayMembershipQuery();
  const oaOwnerQuery = useOaOwnerQuery(
    pubkey,
    // Skip the kind:0 lookup when viewing yourself — the OA gate is for
    // archiving *other* identities you own.
    currentPubkey !== undefined &&
      pubkey.toLowerCase() !== currentPubkey.toLowerCase(),
  );
  const isArchived = useIsIdentityArchived(pubkey);
  const contactListQuery = useContactListQuery(currentPubkey);
  const followMutation = useFollowMutation(currentPubkey);
  const unfollowMutation = useUnfollowMutation(currentPubkey);
  const archiveMutation = useArchiveIdentityMutation();
  const unarchiveMutation = useUnarchiveIdentityMutation();
  const { onOpenAgentSession } = useAgentSession();

  const profile = profileQuery.data;
  const pubkeyLower = pubkey.toLowerCase();
  const presenceStatus = presenceQuery.data?.[pubkeyLower];
  const userStatus = userStatusQuery.data?.[pubkeyLower];

  const relayAgent = relayAgentsQuery.data?.find(
    (a) => a.pubkey.toLowerCase() === pubkeyLower,
  );
  const managedAgent = managedAgentsQuery.data?.find(
    (a) => a.pubkey.toLowerCase() === pubkeyLower,
  );
  const isBot = Boolean(relayAgent || managedAgent);
  const isSelf =
    currentPubkey !== undefined && pubkeyLower === currentPubkey.toLowerCase();
  const canViewActivity = isBot && Boolean(onOpenAgentSession);
  const isFollowing =
    !isSelf &&
    (contactListQuery.data?.contacts.some(
      (contact) => contact.pubkey.toLowerCase() === pubkeyLower,
    ) ??
      false);

  // NIP-IA gates. Button shows when ANY of: self path (acting on own pubkey),
  // admin path (current user is owner/admin in relay_members), or owner path
  // (current user is the verified NIP-OA owner of the viewee per its live
  // kind:0). The relay picks the consent path; we just ensure the request is
  // permitted to be built locally.
  const myRole = myMembershipQuery.data?.role;
  const isRelayAdminOrOwner = myRole === "owner" || myRole === "admin";
  const isOaOwnerOfViewee = oaOwnerQuery.data?.isMe === true;
  const canArchive = isSelf || isRelayAdminOrOwner || isOaOwnerOfViewee;

  const handleArchive = React.useCallback(() => {
    archiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Archived on this relay"),
        onError: (error) =>
          toast.error(
            `Archive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [archiveMutation, pubkey]);

  const handleUnarchive = React.useCallback(() => {
    unarchiveMutation.mutate(
      { targetPubkey: pubkey },
      {
        onSuccess: () => toast.success("Unarchived on this relay"),
        onError: (error) =>
          toast.error(
            `Unarchive failed: ${error instanceof Error ? error.message : String(error)}`,
          ),
      },
    );
  }, [pubkey, unarchiveMutation]);

  const handleCopyPubkey = React.useCallback(() => {
    void navigator.clipboard.writeText(pubkey).then(() => {
      toast.success("Copied to clipboard");
    });
  }, [pubkey]);

  const handleMessage = React.useCallback(() => {
    onOpenDm?.([pubkey]);
    onClose();
  }, [onClose, onOpenDm, pubkey]);

  const displayName = profile?.displayName ?? truncatePubkey(pubkey);

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="user-profile-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : splitPaneClamp
              ? `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`
              : `${widthPx}px`,
        }}
      >
        {!isOverlay && !isSinglePanelView && (
          <button
            aria-label="Resize profile panel"
            className="peer/profile-resize group/profile-resize absolute inset-y-0 left-0 z-40 w-3 -translate-x-1/2 cursor-col-resize"
            data-testid="user-profile-resize-handle"
            onDoubleClick={canResetWidth ? onResetWidth : undefined}
            onPointerDown={onResizeStart}
            title={
              canResetWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover/profile-resize:bg-border/80 group-focus-visible/profile-resize:bg-border/80" />
          </button>
        )}

        {!isOverlay ? (
          <div
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-x-0 top-0 z-40 bg-background/80 backdrop-blur-md after:absolute after:left-0 after:right-0 after:top-10 after:h-px after:bg-border/35 supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
              usesChannelSplitChrome ? "h-[92px]" : "h-[76px]",
            )}
          />
        ) : null}

        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[76px] min-h-[76px] shrink-0 gap-[10px] bg-transparent pb-[4px] pl-[16px] pr-[8px] pt-[42px] sm:pl-[24px] sm:pr-[12px]`
              : isOverlay
                ? "relative z-50 min-h-[44px] shrink-0 gap-3 bg-background/80 px-3 py-[6px] backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55"
                : cn(
                    "absolute inset-x-0 z-50 bg-transparent after:absolute after:bottom-0 after:-left-px after:top-0 after:w-px after:bg-border/45 after:transition-colors peer-hover/profile-resize:after:bg-border/80 peer-focus-visible/profile-resize:after:bg-border/80",
                    usesChannelSplitChrome
                      ? "top-[48px] h-[32px] gap-[10px] py-0 pl-[16px] pr-[8px] sm:pr-[12px]"
                      : "top-[42px] min-h-[32px] gap-3 px-3 py-[4px]",
                  ),
          )}
          data-tauri-drag-region
        >
          <div className="flex min-w-0 items-center gap-1.5">
            <h2
              className={cn(
                "translate-y-px font-semibold tracking-tight",
                usesChannelSplitChrome
                  ? "text-base leading-6"
                  : "text-sm leading-5",
              )}
            >
              Profile
            </h2>
          </div>
          <Button
            aria-label="Close profile"
            className={cn(
              "ml-auto",
              usesChannelSplitChrome
                ? "h-8 w-8 rounded-lg border border-border/40 text-muted-foreground hover:bg-muted/70 hover:text-foreground [&_svg]:size-5"
                : "h-4 w-4 rounded-full text-foreground hover:bg-muted/60 hover:text-foreground",
            )}
            data-testid="user-profile-panel-close"
            onClick={onClose}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X
              className={cn(usesChannelSplitChrome ? "size-5" : "h-2.5 w-2.5")}
            />
          </Button>
        </div>

        <div
          className={cn(
            "min-h-0 flex-1 overflow-y-auto px-4 pb-6",
            !isFloatingOverlay &&
              (usesChannelSplitChrome ? "pt-[92px]" : "pt-[76px]"),
          )}
        >
          <div className="flex flex-col items-center gap-4 pt-4">
            {/* Avatar */}
            {profile?.avatarUrl ? (
              <img
                alt={displayName}
                className="aspect-square w-full rounded-lg object-cover shadow-xs"
                referrerPolicy="no-referrer"
                src={rewriteRelayUrl(profile.avatarUrl)}
              />
            ) : (
              <div className="flex aspect-square w-full items-center justify-center rounded-lg bg-secondary text-5xl font-semibold text-secondary-foreground shadow-xs">
                {displayName.slice(0, 2).toUpperCase()}
              </div>
            )}

            {/* Name + bot identicon */}
            <div className="flex flex-col items-center gap-1">
              <div className="flex items-center gap-2">
                <h3 className="text-base font-semibold">{displayName}</h3>
                {isBot ? (
                  <BotIdenticon
                    value={displayName}
                    size={20}
                    className="shrink-0 rounded"
                  />
                ) : null}
              </div>
              {profile?.nip05Handle ? (
                <p className="text-xs text-muted-foreground">
                  {profile.nip05Handle}
                </p>
              ) : null}
              {/* NIP-IA "Archived" flair (relay-scoped). Spec §Client Behavior:
                  surface archive metadata where relevant. */}
              {isArchived ? (
                <span
                  className="mt-1 inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300"
                  data-testid="user-profile-archived-flair"
                  title="This identity is archived on this relay. Historical events remain attributed to it."
                >
                  <Archive className="h-3 w-3" />
                  Archived on this relay
                </span>
              ) : null}
            </div>

            {/* Presence */}
            {presenceStatus ? <PresenceBadge status={presenceStatus} /> : null}

            {/* User status */}
            {userStatus ? (
              <p className="text-center text-sm text-muted-foreground">
                {userStatus.emoji ? (
                  <StatusEmoji
                    className="mr-1 h-3.5 w-3.5"
                    value={userStatus.emoji}
                  />
                ) : null}
                {userStatus.text}
              </p>
            ) : null}
          </div>

          {/* Pubkey (copyable) */}
          <div className="mt-6">
            <button
              className="flex w-full items-center gap-2 rounded-lg border border-border/60 bg-card/50 px-3 py-2 text-left font-mono text-[11px] text-muted-foreground transition-colors hover:bg-muted/50"
              data-testid="user-profile-copy-pubkey"
              onClick={handleCopyPubkey}
              title="Copy public key"
              type="button"
            >
              <span className="min-w-0 flex-1 truncate">{pubkey}</span>
              <Copy className="h-3.5 w-3.5 shrink-0" />
            </button>
          </div>

          {/* Bot info badges */}
          {isBot && (managedAgent || relayAgent) ? (
            <div className="mt-4 flex flex-wrap gap-1.5">
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

          {/* About */}
          {profile?.about ? (
            <div className="mt-4">
              <h4 className="mb-1 text-xs font-medium uppercase tracking-wider text-muted-foreground/70">
                About
              </h4>
              <p className="text-sm leading-relaxed text-muted-foreground">
                {profile.about}
              </p>
            </div>
          ) : null}

          {/* Actions */}
          <div className="mt-6 flex flex-col gap-2">
            {!isSelf ? (
              isFollowing ? (
                <Button
                  className="w-full"
                  disabled={unfollowMutation.isPending}
                  onClick={() =>
                    unfollowMutation.mutate(pubkey, {
                      onError: (error) =>
                        toast.error(
                          `Unfollow failed: ${error instanceof Error ? error.message : String(error)}`,
                        ),
                    })
                  }
                  type="button"
                  variant="outline"
                >
                  Unfollow
                </Button>
              ) : (
                <Button
                  className="w-full"
                  disabled={followMutation.isPending}
                  onClick={() =>
                    followMutation.mutate(pubkey, {
                      onError: (error) =>
                        toast.error(
                          `Follow failed: ${error instanceof Error ? error.message : String(error)}`,
                        ),
                    })
                  }
                  type="button"
                  variant="default"
                >
                  Follow
                </Button>
              )
            ) : null}
            {onOpenDm && !isSelf ? (
              <Button
                className="w-full"
                data-testid="user-profile-message"
                onClick={handleMessage}
                type="button"
              >
                <MessageSquare className="h-4 w-4" />
                Message
              </Button>
            ) : null}
            {canViewActivity ? (
              <button
                className="flex w-full items-center gap-2 rounded-lg border border-border/60 px-3 py-2 text-left text-xs font-medium text-foreground transition-colors hover:bg-muted/50"
                data-testid={`user-profile-view-activity-${pubkey}`}
                onClick={() => {
                  onClose();
                  onOpenAgentSession?.(pubkey);
                }}
                type="button"
              >
                <Activity className="h-3.5 w-3.5 text-muted-foreground" />
                View activity log
              </button>
            ) : null}
            {/* NIP-IA archive / unarchive. Gated to self / relay admin / OA
                owner of viewee. The relay verifies authority — these gates are
                purely a UX guard. */}
            {canArchive && isArchived === false ? (
              <Button
                className="w-full"
                data-testid="user-profile-archive-identity"
                disabled={archiveMutation.isPending}
                onClick={handleArchive}
                type="button"
                variant="secondary"
              >
                <Archive className="h-4 w-4" />
                {archiveMutation.isPending ? "Archiving…" : "Archive identity"}
              </Button>
            ) : null}
            {canArchive && isArchived === true ? (
              <Button
                className="w-full"
                data-testid="user-profile-unarchive-identity"
                disabled={unarchiveMutation.isPending}
                onClick={handleUnarchive}
                type="button"
                variant="secondary"
              >
                <ArchiveRestore className="h-4 w-4" />
                {unarchiveMutation.isPending
                  ? "Unarchiving…"
                  : "Unarchive identity"}
              </Button>
            ) : null}
          </div>
        </div>
      </aside>
    </>
  );
}
