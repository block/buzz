import * as React from "react";
import { Activity, Copy, MessageSquare, X } from "lucide-react";
import { toast } from "sonner";

import { useUserProfileQuery } from "@/features/profile/hooks";
import {
  useRelayAgentsQuery,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { usePresenceQuery } from "@/features/presence/hooks";
import { useUserStatusQuery } from "@/features/user-status/hooks";
import { PresenceBadge } from "@/features/presence/ui/PresenceBadge";
import { BotIdenticon } from "@/features/messages/ui/BotIdenticon";
import { useAgentSession } from "@/shared/context/AgentSessionContext";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";

type UserProfilePanelProps = {
  canResetWidth: boolean;
  currentPubkey?: string;
  onClose: () => void;
  onOpenDm?: (pubkeys: string[]) => void;
  onResetWidth: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  pubkey: string;
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
  onClose,
  onOpenDm,
  onResetWidth,
  onResizeStart,
  pubkey,
  widthPx,
}: UserProfilePanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay);

  const profileQuery = useUserProfileQuery(pubkey);
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const managedAgentsQuery = useManagedAgentsQuery({ enabled: true });
  const presenceQuery = usePresenceQuery([pubkey]);
  const userStatusQuery = useUserStatusQuery([pubkey]);
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
      {isOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_BASE_CLASS,
          !isOverlay && "pt-11",
          isOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="user-profile-panel"
        style={{ width: `${widthPx}px` }}
      >
        {!isOverlay && (
          <button
            aria-label="Resize profile panel"
            className="group absolute inset-y-0 left-0 z-20 w-3 -translate-x-1/2 cursor-col-resize"
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
            <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80" />
          </button>
        )}

        <div className="flex items-center gap-3 px-4 py-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold tracking-tight">Profile</h2>
          </div>
          <Button
            aria-label="Close profile"
            data-testid="user-profile-panel-close"
            onClick={onClose}
            size="icon"
            type="button"
            variant="ghost"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-6">
          <div className="flex flex-col items-center gap-4 pt-4">
            {/* Avatar */}
            {profile?.avatarUrl ? (
              <img
                alt={displayName}
                className="aspect-square w-full rounded-2xl object-cover shadow-sm"
                referrerPolicy="no-referrer"
                src={rewriteRelayUrl(profile.avatarUrl)}
              />
            ) : (
              <div className="flex aspect-square w-full items-center justify-center rounded-2xl bg-secondary text-5xl font-semibold text-secondary-foreground shadow-sm">
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
            </div>

            {/* Presence */}
            {presenceStatus ? <PresenceBadge status={presenceStatus} /> : null}

            {/* User status */}
            {userStatus ? (
              <p className="text-center text-sm text-muted-foreground">
                {userStatus.emoji ? (
                  <span className="mr-1">{userStatus.emoji}</span>
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
          </div>
        </div>
      </aside>
    </>
  );
}
