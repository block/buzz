import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { useClaimInvite } from "@/features/onboarding/useClaimInvite";
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
 * Claimless links (`buzz://connect`, or a join link without a code) have
 * nothing to confirm against the relay, so the gate renders a static
 * acknowledgment instead: the link is safe in the persisted transaction, and
 * a Continue-setup click records `acknowledged` and returns to the identity
 * steps. The connect itself still runs in `CommunityOnboardingFlow` after
 * machine setup.
 *
 * Rendered as an overlay above `MachineOnboardingFlow` so it can appear and
 * auto-dismiss without losing in-progress identity-step state.
 */
export function PendingInviteGate() {
  const { transaction, update, clear } = useCommunityOnboarding();
  useClaimInvite();

  if (!transaction) return null;

  const isClaimless = !transaction.inviteCode;

  return (
    <div
      className="buzz-onboarding-neutral-theme fixed inset-0 z-50 flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-testid="pending-invite-gate"
    >
      <div className="flex w-full max-w-[440px] flex-col items-center text-center">
        <FlappingBee className="h-auto w-24" />
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">
          {isClaimless ? "Opening community link" : "Opening your invite"}
        </h1>
        {transaction.error ? (
          <p className="mt-3 text-sm leading-6 text-destructive">
            {transaction.error}
          </p>
        ) : isClaimless ? (
          <p className="mt-3 text-sm leading-6 text-muted-foreground">
            You’ll connect to {transaction.communityName} once setup is
            finished.
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
          {isClaimless ? (
            <Button
              className="h-10 w-full"
              data-testid="pending-invite-continue"
              onClick={() => update({ acknowledged: true })}
              type="button"
            >
              Continue setup
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
