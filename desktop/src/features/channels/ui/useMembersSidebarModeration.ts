import * as React from "react";
import { toast } from "sonner";

import {
  useBanMemberMutation,
  useModerationRestrictionsQuery,
  useTimeoutMemberMutation,
  useUnbanMemberMutation,
  useUntimeoutMemberMutation,
} from "@/features/moderation/hooks";
import { useMyRelayMembershipQuery } from "@/features/community-members/hooks";
import { isTimedOut } from "@/features/moderation/lib/restrictionState";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import type { MemberModerationState } from "./MembersSidebarMemberCard";

/**
 * Owns community ban/timeout wiring for the members sidebar. Gated by relay
 * role (owner/admin), independent of the per-channel role — the relay rejects
 * the command events otherwise. Restrictions are only fetched while the sidebar
 * is open and the caller can moderate.
 */
export function useMembersSidebarModeration(open: boolean) {
  const relayMembershipQuery = useMyRelayMembershipQuery();
  const relayRole = relayMembershipQuery.data?.role;
  const canModerate = relayRole === "owner" || relayRole === "admin";
  const restrictionsQuery = useModerationRestrictionsQuery(open && canModerate);
  const banMutation = useBanMemberMutation();
  const unbanMutation = useUnbanMemberMutation();
  const timeoutMutation = useTimeoutMemberMutation();
  const untimeoutMutation = useUntimeoutMemberMutation();
  const isModerationPending =
    banMutation.isPending ||
    unbanMutation.isPending ||
    timeoutMutation.isPending ||
    untimeoutMutation.isPending;

  const [nowMs, setNowMs] = React.useState(() => Date.now());

  // Tick nowMs every second while the sidebar is open — ensures `timedOut`
  // transitions from true→false reactively as TTLs expire without waiting for
  // the next query refresh (staleTime: 15_000).
  React.useEffect(() => {
    if (!open) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [open]);

  const moderationStateByPubkey = React.useMemo(() => {
    const map = new Map<string, MemberModerationState>();
    for (const restriction of restrictionsQuery.data ?? []) {
      map.set(normalizePubkey(restriction.pubkey), {
        banned: restriction.banned,
        timedOut: isTimedOut(restriction.mutedUntil, nowMs),
      });
    }
    return map;
  }, [restrictionsQuery.data, nowMs]);

  const runModerationAction = React.useCallback(
    async (action: () => Promise<unknown>, success: string) => {
      try {
        await action();
        toast.success(success);
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Moderation action failed",
        );
      }
    },
    [],
  );

  const onBan = React.useCallback(
    (member: ChannelMember) =>
      void runModerationAction(
        () => banMutation.mutateAsync({ pubkey: member.pubkey }),
        "Member banned",
      ),
    [banMutation, runModerationAction],
  );

  const onUnban = React.useCallback(
    (member: ChannelMember) =>
      void runModerationAction(
        () => unbanMutation.mutateAsync(member.pubkey),
        "Ban lifted",
      ),
    [unbanMutation, runModerationAction],
  );

  const onTimeout = React.useCallback(
    (member: ChannelMember, expiresAtSecs: number) =>
      void runModerationAction(
        () =>
          timeoutMutation.mutateAsync({
            pubkey: member.pubkey,
            expiresAt: expiresAtSecs,
          }),
        "Member timed out",
      ),
    [timeoutMutation, runModerationAction],
  );

  const onUntimeout = React.useCallback(
    (member: ChannelMember) =>
      void runModerationAction(
        () => untimeoutMutation.mutateAsync(member.pubkey),
        "Timeout lifted",
      ),
    [untimeoutMutation, runModerationAction],
  );

  return {
    canModerate,
    isModerationPending,
    moderationStateByPubkey,
    onBan,
    onUnban,
    onTimeout,
    onUntimeout,
  };
}
