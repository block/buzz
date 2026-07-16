import { AlertTriangle, Info, RefreshCw } from "lucide-react";
import * as React from "react";

import { getNsec } from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { NsecMaskedDisplay } from "./NsecMaskedDisplay";

/**
 * Pure helper so the disabled logic can be unit-tested without a DOM.
 *
 * Disabled while loading (key not fetched yet) or after a failed load (only
 * the explicit "Skip for now" ghost advances past an error).
 */
export function backupNextDisabled({
  isLoading,
  loadError,
}: {
  isLoading: boolean;
  loadError: string | null;
}): boolean {
  return isLoading || loadError !== null;
}

type BackupStepProps = {
  direction: OnboardingTransitionDirection;
  onBack: () => void;
  onNext: () => void;
};

/**
 * Onboarding backup step — shows the user their freshly created key so they
 * can save it somewhere safe. Only shown on the fresh-key path.
 */
export function BackupStep({ direction, onBack, onNext }: BackupStepProps) {
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(true);
  const [loadError, setLoadError] = React.useState<string | null>(null);
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
      className="flex w-full flex-col items-center"
      data-testid="onboarding-page-backup"
      direction={direction}
      transitionKey={`backup-${direction}`}
    >
      <div className="w-full max-w-[500px] text-center">
        <h1 className="text-3xl font-semibold tracking-tight text-foreground">
          Your unique identity has been created
        </h1>
        <p className="mt-3 text-sm leading-6 text-foreground/80">
          This key is stored in your system keychain, but save it some place
          safe in case you ever need to restore your account.
        </p>
      </div>

      <div className="mt-10 w-full max-w-[640px]">
        {isLoading ? (
          <div className="flex items-center justify-center gap-2 py-6 text-sm text-foreground/70">
            <Spinner className="h-4 w-4 border-2" />
            Loading your private key…
          </div>
        ) : loadError ? (
          <div className="mx-auto max-w-[500px] space-y-3 text-left">
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
          <div className="rounded-3xl bg-white/85 px-6 py-5 shadow-[0_0_70px_45px_rgba(255,255,255,0.85)]">
            <NsecMaskedDisplay nsec={nsec} variant="bare" />
          </div>
        ) : (
          <p className="text-center text-sm text-foreground/70">
            No key available to back up.
          </p>
        )}

        {nsec ? (
          <p className="mx-auto mt-6 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-foreground/70">
            <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              Never share your private key. Anyone can impersonate you and
              access everything in your account.
            </span>
          </p>
        ) : null}
      </div>

      <div className="mt-12 flex flex-col items-center gap-3">
        <Button
          className="h-10 rounded-full px-8"
          data-testid="onboarding-next"
          disabled={backupNextDisabled({ isLoading, loadError })}
          onClick={onNext}
          type="button"
        >
          Next
        </Button>

        {loadError ? (
          <Button
            className="h-9 rounded-full px-5 text-muted-foreground hover:text-accent-foreground"
            data-testid="backup-skip"
            onClick={onNext}
            type="button"
            variant="ghost"
          >
            Skip for now
          </Button>
        ) : null}

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
          data-testid="onboarding-back"
          onClick={onBack}
          type="button"
          variant="ghost"
        >
          Back
        </Button>
      </div>
    </OnboardingSlideTransition>
  );
}
