import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  communityThreadPolicyQueryKey,
  useCommunityThreadPolicyQuery,
} from "@/features/communities/useCommunityThreadPolicy";
import {
  setCommunityThreadPolicy,
  type ThreadPolicy,
} from "@/shared/api/communityProfile";
import { SegmentedControl } from "@/shared/ui/segmented-control";

const THREAD_POLICY_OPTIONS = [
  { value: "nested", label: "Nested" },
  { value: "flat", label: "One level" },
] as const satisfies readonly { value: ThreadPolicy; label: string }[];

/**
 * Admin control for the active community's NIP-TP thread policy. Publishes a
 * kind:9034 command; render it only for relay admins and owners.
 */
export function CommunityThreadPolicySettingsCard() {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl;
  const policyQuery = useCommunityThreadPolicyQuery();
  const queryClient = useQueryClient();
  const mutation = useMutation({
    mutationFn: (policy: ThreadPolicy) => setCommunityThreadPolicy(policy),
    onSuccess: async (_data, policy) => {
      if (!relayUrl) return;
      queryClient.setQueryData(communityThreadPolicyQueryKey(relayUrl), policy);
      await queryClient.invalidateQueries({
        queryKey: communityThreadPolicyQueryKey(relayUrl),
      });
    },
    onError: (error) => {
      toast.error(
        error instanceof Error
          ? error.message
          : "Couldn’t update thread replies.",
      );
    },
  });
  const value =
    (mutation.isPending ? mutation.variables : undefined) ??
    policyQuery.data ??
    "nested";

  return (
    <div
      className="flex items-center justify-between gap-4 rounded-xl border border-border/70 bg-muted/20 p-3"
      data-testid="community-thread-policy-settings"
    >
      <div className="min-w-0">
        <p className="text-sm font-medium">Thread replies</p>
        <p className="text-xs text-muted-foreground">
          One level keeps every reply directly under the thread’s first message.
        </p>
      </div>
      <SegmentedControl
        disabled={mutation.isPending || policyQuery.data === undefined}
        legend="Thread replies"
        onValueChange={(policy) => {
          if (policy !== value) mutation.mutate(policy);
        }}
        optionTestIdPrefix="community-thread-policy"
        options={THREAD_POLICY_OPTIONS}
        size="compact"
        testId="community-thread-policy-control"
        value={value}
      />
    </div>
  );
}
