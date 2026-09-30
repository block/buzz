import * as React from "react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import {
  inviteErrorMessage,
  isInviteExhaustedError,
  isInviteExpiredError,
} from "@/shared/api/inviteHelpers";
import { claimInvite } from "@/shared/api/invites";

/**
 * Drive the `claiming` stage after machine onboarding completes: claim the
 * invite with the user's final identity, then advance to `connecting`.
 * Completion is fenced by transaction ID so cancelling or replacing the
 * transaction while the request is pending cannot mutate the replacement.
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
    void claimInvite(
      transaction.relayUrl,
      transaction.inviteCode ?? "",
      transaction.policyReceipt,
    )
      .then(() => {
        update({ stage: "connecting", error: undefined }, transaction.id);
      })
      .catch((error: unknown) =>
        update(
          {
            error: isInviteExpiredError(error)
              ? "This invite code has expired — ask for a new one."
              : isInviteExhaustedError(error)
                ? "This invite has reached its use limit. Ask for a new invite."
                : inviteErrorMessage(error),
          },
          transaction.id,
        ),
      )
      .finally(() => setIsPending(false));
  }, [isPending, transaction, update]);
}
