import * as React from "react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import { claimInvite } from "@/shared/api/invites";
import { getIdentity } from "@/shared/api/tauriIdentity";

/**
 * Drive the `claiming` stage of the community-onboarding transaction: claim
 * the invite against its relay, then advance to `connecting` with the pubkey
 * that signed the claim recorded as `claimedPubkey`. Shared by
 * `PendingInviteGate` (claims during machine onboarding, possibly with the
 * boot-time key the identity steps later replace) and
 * `CommunityOnboardingFlow` (which compares `claimedPubkey` against the final
 * identity and re-claims on mismatch) — the two mount mutually exclusively,
 * gate before machine onboarding completes, flow after, so only one claim
 * runs at a time.
 *
 * The error guard keeps a failed claim parked on the caller's Retry
 * affordance — without it the effect refires on the error-bearing transaction
 * and re-claims in a loop.
 */
export function useClaimInvite() {
  const { transaction, update } = useCommunityOnboarding();
  const [isPending, setIsPending] = React.useState(false);

  React.useEffect(() => {
    if (transaction?.stage !== "claiming" || transaction.error || isPending) {
      return;
    }
    setIsPending(true);
    void getIdentity()
      .then(async (identity) => {
        await claimInvite(transaction.relayUrl, transaction.inviteCode ?? "");
        update({
          stage: "connecting",
          error: undefined,
          claimedPubkey: identity.pubkey,
        });
      })
      .catch((error: unknown) => update({ error: inviteErrorMessage(error) }))
      .finally(() => setIsPending(false));
  }, [isPending, transaction, update]);
}
