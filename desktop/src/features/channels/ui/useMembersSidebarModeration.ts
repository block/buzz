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
import {
  hasObservableTimeout,
  isTimedOut,
} from "@/features/moderation/lib/restrictionState";
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

  // Tick nowMs every second only while there is a live timeout to count down —
  // ensures `timedOut` transitions from true→false reactively as TTLs expire
  // without waiting for the next query refresh (staleTime: 15_000). Gating on
  // an observable timeout keeps a large member list from reconciling every card
  // at 1 Hz when nothing is expiring. Termination is self-consistent: the last
  // expiry's next tick advances nowMs past it, `shouldTick` flips false, and
  // cleanup clears the interval — that same tick is what flips the card to
  // "not timed out".
  const shouldTick =
    open && canModerate && hasObservableTimeout(restrictionsQuery.data, nowMs);
  React.useEffect(() => {
    if (!shouldTick) return;
    const id = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [shouldTick]);

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
