import * as React from "react";
import { toast } from "sonner";

import {
  useBanMemberMutation,
  useModerationRestrictionsQuery,
  useTimeoutMemberMutation,
  useUnbanMemberMutation,
  useUntimeoutMemberMutation,
} from "@/features/moderation/hooks";
import { useMyRelayMembershipQuery } from "@/features/relay-members/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import type { MemberModerationState } from "./MembersSidebarMemberCard";

/**
 * Coerce a restriction timestamp to epoch milliseconds. The wire emits
 * DateTime<Utc> as an RFC3339 string, but the shared type still tolerates the
 * legacy `number` (unix seconds) shape, so handle both: strings parse as ISO,
 * numbers are treated as unix seconds. Returns null for absent/unparseable
 * values (fails closed — no phantom "timed out" state).
 */
function parseTimestampMs(value: string | number | null): number | null {
  if (value == null) return null;
  if (typeof value === "number") return value * 1000;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

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

  const moderationStateByPubkey = React.useMemo(() => {
    const nowMs = Date.now();
    const map = new Map<string, MemberModerationState>();
    for (const restriction of restrictionsQuery.data ?? []) {
      const mutedUntilMs = parseTimestampMs(restriction.mutedUntil);
      const timedOut = mutedUntilMs != null && mutedUntilMs > nowMs;
      map.set(normalizePubkey(restriction.pubkey), {
        banned: restriction.banned,
        timedOut,
      });
    }
    return map;
  }, [restrictionsQuery.data]);

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
