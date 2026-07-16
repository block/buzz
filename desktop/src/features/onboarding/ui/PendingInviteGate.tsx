import * as React from "react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import { claimInvite } from "@/shared/api/invites";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";

/**
 * Full-screen loading state for an invite (`buzz://join`) deep link that
 * arrives before machine onboarding is complete: connect to the invite's
 * relay right away to confirm the invite is real, then drop back into the
 * identity steps automatically. The membership claim it performs is the same
 * call `CommunityOnboardingFlow` makes when a link arrives after machine
 * onboarding — on success the persisted transaction advances to
 * `connecting`, and the rest of the join resumes once setup finishes.
 *
 * Rendered as an overlay above `MachineOnboardingFlow` so it can appear and
 * auto-dismiss without losing in-progress identity-step state.
 */
export function PendingInviteGate() {
  const { transaction, update, clear } = useCommunityOnboarding();
  const [isPending, setIsPending] = React.useState(false);

  React.useEffect(() => {
    if (transaction?.stage !== "claiming" || transaction.error || isPending) {
      return;
    }
    setIsPending(true);
    // Record which key signed the claim: this gate runs before the identity
    // steps behind it are done, so the user can still import a different key.
    // CommunityOnboardingFlow re-claims after setup if the final key differs.
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

  if (!transaction) return null;

  return (
    <div
      className="buzz-onboarding-neutral-theme fixed inset-0 z-50 flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-testid="pending-invite-gate"
    >
      <div className="flex w-full max-w-[440px] flex-col items-center text-center">
        <FlappingBee className="h-auto w-24" />
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">
          Opening your invite
        </h1>
        {transaction.error ? (
          <p className="mt-3 text-sm leading-6 text-destructive">
            {transaction.error}
          </p>
        ) : null}
        <div className="mt-8 flex w-full flex-col gap-3">
          {transaction.error ? (
            <Button
              className="h-10 w-full"
              data-testid="pending-invite-retry"
              onClick={() => update({ error: undefined })}
              type="button"
            >
              Retry
            </Button>
          ) : null}
          <Button
            className="h-10 w-full"
            data-testid="pending-invite-cancel"
            onClick={clear}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
        </div>
      </div>
    </div>
  );
}
