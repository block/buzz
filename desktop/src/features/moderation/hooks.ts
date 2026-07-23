import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { getRelaySelf } from "@/features/moderation/lib/relaySelf";
import {
  banMember,
  type CommunityRestriction,
  listAuditActions,
  listReports,
  listRestrictions,
  type ModerationAction,
  type ModerationReport,
  type ReportType,
  type ResolutionAction,
  type ResolutionStatus,
  resolveReport,
  submitReport,
  timeoutMember,
  unbanMember,
  untimeoutMember,
} from "@/shared/api/moderation";

export const moderationReportsQueryKey = ["moderationReports"] as const;
export const moderationAuditQueryKey = ["moderationAudit"] as const;
export const moderationRestrictionsQueryKey = [
  "moderationRestrictions",
] as const;
export const relaySelfQueryKey = ["relaySelf"] as const;

/**
 * The active relay's NIP-11 `self` pubkey (hex), or `null` when it advertises
 * none. Used to recognize relay-signed state and moderation DMs. Community-
 * scoped and effectively static for a session, so it is cached indefinitely;
 * a `null` result is a valid answer, while request failures remain query errors.
 */
export function useRelaySelfQuery(enabled = true) {
  return useQuery({
    enabled,
    queryKey: relaySelfQueryKey,
    queryFn: getRelaySelf,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

// --- Reads (mod-authz gated; consumed by the U2 queue/audit surfaces) ---

export function useModerationReportsQuery(
  options?: { status?: string; limit?: number },
  enabled = true,
) {
  return useQuery({
    enabled,
    queryKey: [
      ...moderationReportsQueryKey,
      options?.status ?? null,
      options?.limit ?? null,
    ],
    queryFn: () => listReports(options),
    staleTime: 15_000,
  });
}

export function useModerationAuditQuery(limit?: number, enabled = true) {
  return useQuery({
    enabled,
    queryKey: [...moderationAuditQueryKey, limit ?? null],
    queryFn: () => listAuditActions(limit),
    staleTime: 15_000,
  });
}

export function useModerationRestrictionsQuery(enabled = true) {
  return useQuery({
    enabled,
    queryKey: moderationRestrictionsQueryKey,
    queryFn: listRestrictions,
    staleTime: 15_000,
  });
}

// --- Writes ---
//
// Moderation writes are relay-validated command events whose effects surface in
// the queue/audit/restricted reads after processing, so mutations invalidate the
// affected read queries on success rather than fabricating optimistic rows.

function useInvalidateModerationReads() {
  const queryClient = useQueryClient();
  return () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: moderationReportsQueryKey }),
      queryClient.invalidateQueries({ queryKey: moderationAuditQueryKey }),
      queryClient.invalidateQueries({
        queryKey: moderationRestrictionsQueryKey,
      }),
    ]);
}

/** Submit a NIP-56 report. Does not touch the mod-gated read caches. */
export function useSubmitReportMutation() {
  return useMutation({
    mutationFn: (input: {
      authorPubkey: string;
      eventId: string;
      reportType: ReportType;
      note?: string;
    }) => submitReport(input),
  });
}

export function useBanMemberMutation() {
  const invalidate = useInvalidateModerationReads();
  return useMutation({
    mutationFn: (input: {
      pubkey: string;
      expiresAt?: number;
      reason?: string;
    }) => banMember(input),
    onSuccess: invalidate,
  });
}

export function useUnbanMemberMutation() {
  const invalidate = useInvalidateModerationReads();
  return useMutation({
    mutationFn: (pubkey: string) => unbanMember(pubkey),
    onSuccess: invalidate,
  });
}

export function useTimeoutMemberMutation() {
  const invalidate = useInvalidateModerationReads();
  return useMutation({
    mutationFn: (input: {
      pubkey: string;
      expiresAt: number;
      reason?: string;
    }) => timeoutMember(input),
    onSuccess: invalidate,
  });
}

export function useUntimeoutMemberMutation() {
  const invalidate = useInvalidateModerationReads();
  return useMutation({
    mutationFn: (pubkey: string) => untimeoutMember(pubkey),
    onSuccess: invalidate,
  });
}

export function useResolveReportMutation() {
  const invalidate = useInvalidateModerationReads();
  return useMutation({
    mutationFn: (input: {
      reportEventId: string;
      status: ResolutionStatus;
      action: ResolutionAction;
      reason?: string;
    }) => resolveReport(input),
    onSuccess: invalidate,
  });
}

export type {
  CommunityRestriction,
  ModerationAction,
  ModerationReport,
  ReportType,
  ResolutionAction,
  ResolutionStatus,
};
