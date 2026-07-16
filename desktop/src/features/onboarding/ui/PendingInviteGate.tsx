import { Button } from "@/shared/ui/button";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";

/**
 * Full-screen acknowledgment for an invite (`buzz://join`) or community
 * (`buzz://connect`) deep link that arrives before machine onboarding is
 * complete. The claim itself has to wait for the identity steps — this gate
 * exists so opening an invite link on a fresh install visibly reacts instead
 * of silently queueing the link behind "Welcome to Buzz".
 *
 * Rendered as an overlay above `MachineOnboardingFlow` so dismissing it
 * returns to the identity steps without losing their in-progress state.
 */
export function PendingInviteGate({
  communityName,
  hasInviteCode,
  onContinue,
}: {
  communityName: string;
  hasInviteCode: boolean;
  onContinue: () => void;
}) {
  return (
    <div
      className="buzz-onboarding-neutral-theme fixed inset-0 z-50 flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-testid="pending-invite-gate"
    >
      <div className="flex w-full max-w-[440px] flex-col items-center text-center">
        <FlappingBee className="h-auto w-24" />
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">
          {hasInviteCode ? "Opening your invite" : "Opening community link"}
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          {hasInviteCode
            ? `You've been invited to join ${communityName}.`
            : `You're connecting to ${communityName}.`}{" "}
          Finish setting up Buzz and it will open automatically.
        </p>
        <Button
          className="mt-8 h-10 w-full"
          data-testid="pending-invite-continue"
          onClick={onContinue}
          type="button"
        >
          Continue setup
        </Button>
      </div>
    </div>
  );
}
