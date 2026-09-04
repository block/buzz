import type { LucideIcon } from "lucide-react";
import type { ReactNode, Ref } from "react";
import {
  Hand,
  Headphones,
  MessageSquare,
  Play,
  RefreshCw,
  Square,
  UserMinus,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import type {
  useFollowMutation,
  useUnfollowMutation,
} from "@/features/profile/hooks";
import { useFeatureEnabled } from "@/shared/features";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

// ── Primary actions ──────────────────────────────────────────────────────────

export function ProfilePrimaryActions({
  actionGroupRef,
  agentActionDisabled,
  agentStartBlockReason,
  agentActionLabel,
  agentActionLive,
  className,
  concealed = false,
  followMutation,
  huddlePending,
  isFollowing,
  messagePending,
  onAgentPrimaryAction,
  onAgentRestart,
  onHuddle,
  onMessage,
  onWave,
  pubkey,
  unfollowMutation,
  wavePending,
}: {
  actionGroupRef?: Ref<HTMLDivElement>;
  agentActionDisabled?: boolean;
  agentStartBlockReason?: string;
  agentActionLabel?: string;
  agentActionLive?: boolean;
  className?: string;
  concealed?: boolean;
  followMutation: ReturnType<typeof useFollowMutation>;
  huddlePending?: boolean;
  isFollowing: boolean;
  messagePending?: boolean;
  onAgentPrimaryAction?: () => void;
  onAgentRestart?: () => void;
  onHuddle?: () => void;
  onMessage?: () => void;
  onWave?: () => void;
  pubkey: string;
  unfollowMutation: ReturnType<typeof useUnfollowMutation>;
  wavePending?: boolean;
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

  const hasActions =
    showFollowAction ||
    onWave !== undefined ||
    onMessage !== undefined ||
    onHuddle !== undefined ||
    (onAgentPrimaryAction !== undefined && agentActionLabel !== undefined) ||
    onAgentRestart !== undefined;

  if (!hasActions) {
    return null;
  }

  return (
    <ProfileActionGroup
      className={className}
      concealed={concealed}
      ref={actionGroupRef}
    >
      {onAgentPrimaryAction && agentActionLabel ? (
        <ProfileActionTile
          active
          disabled={agentActionDisabled || Boolean(agentStartBlockReason)}
          title={agentStartBlockReason}
          icon={agentActionLive ? Square : Play}
          label={agentActionLabel}
          onClick={onAgentPrimaryAction}
          testId="user-profile-agent-primary-action"
        />
      ) : null}
      {onAgentRestart ? (
        <ProfileActionTile
          disabled={agentActionDisabled}
          icon={RefreshCw}
          label="Restart agent"
          onClick={onAgentRestart}
          testId="user-profile-agent-restart"
        />
      ) : null}
      {onMessage ? (
        <ProfileActionTile
          disabled={messagePending}
          icon={MessageSquare}
          isLoading={messagePending}
          label="Message"
          onClick={onMessage}
          testId="user-profile-message"
        />
      ) : null}
      {onHuddle ? (
        <ProfileActionTile
          disabled={huddlePending}
          icon={Headphones}
          isLoading={huddlePending}
          label="Huddle"
          onClick={onHuddle}
          testId="user-profile-huddle"
        />
      ) : null}
      {onWave ? (
        <ProfileActionTile
          disabled={wavePending}
          icon={Hand}
          isLoading={wavePending}
          label="Wave"
          onClick={onWave}
          testId="user-profile-wave"
        />
      ) : null}
      {showFollowAction ? (
        <ProfileActionTile
          active={isFollowing}
          disabled={followToggleMutation.isPending}
          icon={isFollowing ? UserMinus : UserPlus}
          label={isFollowing ? "Unfollow" : "Follow"}
          onClick={handleFollowClick}
        />
      ) : null}
    </ProfileActionGroup>
  );
}

export function ProfilePersonaPrimaryActions({
  actionGroupRef,
  className,
  concealed = false,
  disabled,
  onStartAgent,
}: {
  actionGroupRef?: Ref<HTMLDivElement>;
  className?: string;
  concealed?: boolean;
  disabled: boolean;
  onStartAgent: () => void;
}) {
  return (
    <ProfileActionGroup
      className={className}
      concealed={concealed}
      ref={actionGroupRef}
    >
      <ProfileActionTile
        active
        disabled={disabled}
        icon={Play}
        label="Start agent"
        onClick={onStartAgent}
        testId="user-profile-start-agent"
      />
    </ProfileActionGroup>
  );
}

function ProfileActionGroup({
  children,
  className,
  concealed,
  ref,
}: {
  children: ReactNode;
  className?: string;
  concealed: boolean;
  ref?: Ref<HTMLDivElement>;
}) {
  return (
    <div
      aria-hidden={concealed || undefined}
      className={cn("grid grid-flow-col auto-cols-fr gap-2", className)}
      data-testid="user-profile-primary-actions"
      inert={concealed || undefined}
      ref={ref}
    >
      {children}
    </div>
  );
}

function ProfileActionTile({
  active,
  disabled,
  icon: Icon,
  isLoading,
  label,
  onClick,
  testId,
  title,
}: {
  active?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  isLoading?: boolean;
  label: string;
  onClick: () => void;
  testId?: string;
  title?: string;
}) {
  return (
    <button
      aria-label={label}
      aria-busy={isLoading || undefined}
      className={cn(
        "flex min-h-20 w-full flex-col items-center justify-center gap-1.5 rounded-xl bg-muted px-2 py-3 text-center transition-colors hover:bg-muted/80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
        active && "bg-foreground text-background hover:bg-foreground/90",
      )}
      data-testid={testId}
      title={title}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      {isLoading ? (
        <Spinner aria-hidden="true" className="h-5 w-5 border-2" />
      ) : (
        <Icon
          className={cn("h-5 w-5 text-foreground", active && "text-background")}
        />
      )}
      <span className="min-w-0 text-xs font-medium leading-tight">{label}</span>
    </button>
  );
}
