import { AlertTriangle, RefreshCw } from "lucide-react";
import * as React from "react";

import { getNsec } from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { StepProgress } from "@/shared/ui/step-progress";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { NsecMaskedDisplay } from "./NsecMaskedDisplay";

/**
 * Pure helper so the disabled logic can be unit-tested without a DOM.
 *
 * Disabled when:
 * - still loading (key not fetched yet)
 * - load failed (only the explicit "Skip for now" ghost advances past an error)
 * - key loaded AND checkbox not yet checked
 *
 * Enabled when key is null after a clean (non-error) load — backend returned
 * nothing, so there is nothing to acknowledge and the user may proceed.
 */
export function backupNextDisabled({
  isLoading,
  loadError,
  nsec,
  hasAcknowledged,
}: {
  isLoading: boolean;
  loadError: string | null;
  nsec: string | null;
  hasAcknowledged: boolean;
}): boolean {
  return isLoading || loadError !== null || (nsec !== null && !hasAcknowledged);
}

type BackupStepProps = {
  currentStep: number;
  direction: OnboardingTransitionDirection;
  totalSteps: number;
  onBack: () => void;
  onNext: () => void;
};

/**
 * Onboarding backup step — shows the user their nsec and requires
 * acknowledgement before advancing. Only shown on the fresh-key path.
 */
export function BackupStep({
  currentStep,
  direction,
  totalSteps,
  onBack,
  onNext,
}: BackupStepProps) {
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
  const [hasAcknowledged, setHasAcknowledged] = React.useState(false);
  const cancelledRef = React.useRef(false);

  const loadNsec = React.useCallback(async () => {
    setIsLoading(true);
    setLoadError(null);
    try {
      const value = await getNsec();
      if (!cancelledRef.current) setNsec(value);
    } catch (err) {
      if (!cancelledRef.current)
        setLoadError(
          err instanceof Error
            ? err.message
            : "Failed to retrieve private key.",
        );
    } finally {
      if (!cancelledRef.current) setIsLoading(false);
    }
  }, []);

  React.useEffect(() => {
    cancelledRef.current = false;
    void loadNsec();
    return () => {
      // Back-during-fetch: cancel any in-flight setState calls and clear the
      // nsec from memory on unmount (backup step is only on the fresh-key path).
      cancelledRef.current = true;
      setNsec(null);
    };
  }, [loadNsec]);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center pb-36 lg:pb-0"
      data-testid="onboarding-page-backup"
      direction={direction}
      transitionKey={`backup-${direction}`}
    >
      <div className="w-full max-w-[500px] text-center">
        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          Save your private key
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Buzz generated a Nostr private key for you. This key is stored in your
          system keychain, but you should also save it somewhere safe in case
          you ever need to restore your account on another device.
        </p>
      </div>

      <div className="mt-8 w-full max-w-[500px] space-y-4 text-left">
        {isLoading ? (
          <div className="flex items-center gap-2 rounded-lg border border-border/70 bg-muted/30 px-4 py-6 text-sm text-muted-foreground">
            <Spinner className="h-4 w-4 border-2" />
            Loading your private key…
          </div>
        ) : loadError ? (
          <div className="space-y-3">
            <div
              className="flex items-start gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
              data-testid="backup-load-error"
            >
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
              <span>
                Could not retrieve your private key: {loadError}. You can
                continue and find it later in Settings &gt; Profile &gt;
                Identity.
              </span>
            </div>
            <Button
              className="h-8 gap-1.5 text-sm"
              data-testid="backup-retry"
              onClick={() => void loadNsec()}
              size="sm"
              type="button"
              variant="outline"
            >
              <RefreshCw className="h-3.5 w-3.5" />
              Try again
            </Button>
          </div>
        ) : nsec ? (
          <NsecMaskedDisplay nsec={nsec} />
        ) : (
          <p className="text-sm text-muted-foreground">
            No key available to back up.
          </p>
        )}

        {nsec ? (
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm leading-5 text-amber-700 dark:text-amber-400">
            <strong>Never share your private key.</strong> Anyone with it can
            impersonate you and access everything in your account.
          </div>
        ) : null}

        {!isLoading && !loadError && nsec ? (
          <label className="flex cursor-pointer items-start gap-3">
            <input
              checked={hasAcknowledged}
              className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-primary"
              data-testid="backup-acknowledge"
              onChange={(e) => setHasAcknowledged(e.target.checked)}
              type="checkbox"
            />
            <span className="text-sm leading-5">
              I've saved my private key in a safe place
            </span>
          </label>
        ) : null}
      </div>

      <div className="mt-8 flex w-full max-w-[500px] flex-col gap-3 max-lg:pointer-events-none max-lg:fixed max-lg:inset-x-0 max-lg:bottom-0 max-lg:z-40 max-lg:mt-0 max-lg:max-w-none max-lg:border-t max-lg:border-border max-lg:bg-background max-lg:p-4 max-lg:pb-[max(1rem,env(safe-area-inset-bottom))]">
        <Button
          className="h-10 w-full max-lg:pointer-events-auto"
          data-testid="onboarding-next"
          disabled={backupNextDisabled({
            isLoading,
            loadError,
            nsec,
            hasAcknowledged,
          })}
          onClick={onNext}
          type="button"
        >
          Next
        </Button>

        {loadError ? (
          <Button
            className="h-10 w-full text-muted-foreground hover:text-accent-foreground max-lg:pointer-events-auto"
            data-testid="backup-skip"
            onClick={onNext}
            type="button"
            variant="ghost"
          >
            Skip for now
          </Button>
        ) : null}

        <Button
          className="h-10 w-full text-muted-foreground hover:text-accent-foreground max-lg:pointer-events-auto"
          data-testid="onboarding-back"
          onClick={onBack}
          type="button"
          variant="ghost"
        >
          Back
        </Button>

        <StepProgress
          activeSegmentClassName="bg-primary"
          className="mt-1 max-lg:pointer-events-auto lg:hidden"
          completeSegmentClassName="bg-primary/35"
          currentStep={currentStep}
          inactiveSegmentClassName="bg-muted-foreground/25"
          totalSteps={totalSteps}
        />
      </div>
    </OnboardingSlideTransition>
  );
}
