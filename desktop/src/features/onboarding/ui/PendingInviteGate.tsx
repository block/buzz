import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

/**
 * Acknowledge a community deep link received before machine onboarding is
 * complete. The transaction is already persisted; claiming and connecting
 * wait until setup finishes so only the user's final identity is admitted.
 */
export function PendingInviteGate() {
  const { transaction, update, clear } = useCommunityOnboarding();
  const systemColorScheme = useSystemColorScheme();

  if (!transaction) return null;

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell fixed inset-0 z-50 flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
      data-testid="pending-invite-gate"
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        <FlappingBee className="h-auto w-24" />
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">
          Opening community link
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          You’ll connect to {transaction.communityName} once setup is finished.
        </p>
        <div className="mt-8 flex w-full max-w-[300px] flex-col gap-3">
          <Button
            className="h-10 w-full"
            data-testid="pending-invite-continue"
            onClick={() => update({ acknowledged: true })}
            type="button"
          >
            Continue setup
          </Button>
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
