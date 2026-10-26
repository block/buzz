import * as React from "react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import {
  inviteClaimErrorMessage,
  inviteErrorMessage,
} from "@/shared/api/inviteHelpers";
import { claimInvite, getJoinPolicy } from "@/shared/api/invites";

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
  const currentTransaction = React.useRef(transaction);
  React.useEffect(() => {
    currentTransaction.current = transaction;
    return () => {
      currentTransaction.current = null;
    };
  }, [transaction]);

  React.useEffect(() => {
    if (transaction?.stage !== "claiming" || transaction.error || isPending) {
      return;
    }
    setIsPending(true);
    void (async () => {
      const policy = !transaction.policyReceipt
        ? await getJoinPolicy(transaction.relayUrl, "webview")
        : null;
      if (currentTransaction.current !== transaction) return;
      if (policy) {
        update({ stage: "invite-consent" }, transaction.id);
        return;
      }
      await claimInvite(
        transaction.relayUrl,
        transaction.inviteCode ?? "",
        transaction.policyReceipt,
      );
      update({ stage: "connecting", error: undefined }, transaction.id);
    })()
      .catch((error: unknown) => {
        if (currentTransaction.current !== transaction) return;
        if (inviteErrorMessage(error).includes("join_policy_required")) {
          update(
            {
              stage: "invite-consent",
              policyReceipt: undefined,
              error: undefined,
            },
            transaction.id,
          );
          return;
        }
        update(
          {
            error: inviteClaimErrorMessage(error),
          },
          transaction.id,
        );
      })
      .finally(() => setIsPending(false));
  }, [isPending, transaction, update]);
}
