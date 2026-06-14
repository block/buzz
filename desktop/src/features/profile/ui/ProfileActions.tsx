import type { LucideIcon } from "lucide-react";
import {
  ChevronRight,
  MessageSquare,
  Pencil,
  UserMinus,
  UserPlus,
} from "lucide-react";
import { toast } from "sonner";

import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import type {
  useFollowMutation,
  useUnfollowMutation,
} from "@/features/profile/hooks";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Badge } from "@/shared/ui/badge";

export function ProfileWorkingBadge({
  channelId,
  name,
  observedAt,
  onNavigate,
}: {
  channelId: string;
  name: string;
  observedAt: number;
  onNavigate: (channelId: string) => void;
}) {
  const now = useNow(1000);

  return (
    <Badge
      className="cursor-pointer motion-safe:animate-pulse normal-case tracking-normal hover:opacity-80"
      variant="default"
      onClick={() => onNavigate(channelId)}
    >
      Working in #{name} · {formatElapsed(now - observedAt)}
    </Badge>
  );
}

export function ProfilePrimaryActions({
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
  return (
    <div className="flex items-start justify-center gap-8">
      {isFollowing ? (
        <ProfileQuickAction
          active
          disabled={unfollowMutation.isPending}
          icon={UserMinus}
          label="Unfollow"
          onClick={() =>
            unfollowMutation.mutate(pubkey, {
              onError: (error) =>
                toast.error(
                  `Unfollow failed: ${error instanceof Error ? error.message : String(error)}`,
                ),
            })
          }
        />
      ) : (
        <ProfileQuickAction
          disabled={followMutation.isPending}
          icon={UserPlus}
          label="Follow"
          onClick={() =>
            followMutation.mutate(pubkey, {
              onError: (error) =>
                toast.error(
                  `Follow failed: ${error instanceof Error ? error.message : String(error)}`,
                ),
            })
          }
        />
      )}
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
        <Icon className="h-5 w-5" />
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

export function ProfileIngressRow({
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
