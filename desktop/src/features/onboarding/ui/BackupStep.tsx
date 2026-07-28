import { AlertTriangle, Info, RefreshCw } from "lucide-react";
import * as React from "react";

import { getNsec } from "@/shared/api/tauriIdentity";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Spinner } from "@/shared/ui/spinner";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { EncryptedBackupCreator } from "./EncryptedBackupCreator";
import { NsecMaskedDisplay } from "./NsecMaskedDisplay";

export type BackupStepMode = "encrypted" | "raw";

/** Saving a Keycase is recommended, never required to continue onboarding. */
export function backupNextDisabled(): boolean {
  return false;
}

type BackupStepProps = {
  direction: OnboardingTransitionDirection;
  onBack: () => void;
  onNext: () => void;
};

/**
 * Onboarding backup step — recommends a portable Keycase without blocking
 * setup. The raw key is fetched only after the user chooses the advanced path.
 */
export function BackupStep({ direction, onBack, onNext }: BackupStepProps) {
  const [mode, setMode] = React.useState<BackupStepMode>("encrypted");
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
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
    return () => {
      // Back-during-fetch: cancel any in-flight setState calls and clear the
      // nsec from memory on unmount (backup step is only on the fresh-key path).
      cancelledRef.current = true;
      setNsec(null);
    };
  }, []);

  const showRawKey = React.useCallback(() => {
    setMode("raw");
    void loadNsec();
  }, [loadNsec]);

  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-center"
      data-testid="onboarding-page-backup"
      direction={direction}
      transitionKey={`backup-${direction}`}
    >
      <div className="flex w-full max-w-[500px] shrink-0 flex-col text-center">
        <h1 className="text-title font-normal text-foreground">
          Account created!
        </h1>
        <p className="mt-5 text-sm leading-6 text-foreground/80">
          {mode === "encrypted"
            ? "Buzz keeps your identity in the system keychain. Save a portable, password-protected Keycase in case you need to restore it elsewhere."
            : "This key is stored in your system keychain, but save it some place safe in case you ever need to restore your account."}
        </p>
      </div>

      <div className="flex w-full max-w-[1040px] flex-1 flex-col justify-center py-10">
        {mode === "encrypted" ? (
          <Card className="w-full px-8 py-6" variant="textured">
            <div className="mx-auto w-full max-w-[832px]">
              <div className="mb-5 space-y-2 text-center">
                <h2 className="text-lg font-medium">Save a Keycase</h2>
                <p className="text-xs leading-5 text-muted-foreground">
                  A Keycase is still private. Never publish or share it. You
                  need both the file and password to restore your identity.
                </p>
              </div>
              <EncryptedBackupCreator variant="spotlight" />
            </div>
          </Card>
        ) : isLoading ? (
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
          <Card className="w-full px-8 py-6" variant="textured">
            <div className="mx-auto w-full max-w-[832px]">
              <NsecMaskedDisplay nsec={nsec} variant="bare" />
            </div>
          </Card>
        ) : (
          <p className="text-center text-sm text-foreground/70">
            No key available to back up.
          </p>
        )}

        {mode === "encrypted" ? (
          <div className="mt-4 flex justify-center">
            <Button
              className="h-8 text-sm text-muted-foreground hover:text-accent-foreground"
              data-testid="backup-show-raw-key"
              onClick={showRawKey}
              size="sm"
              type="button"
              variant="ghost"
            >
              Advanced: show my private key instead
            </Button>
          </div>
        ) : null}

        {mode === "raw" && nsec ? (
          <p className="mx-auto mt-6 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
            <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>
              Never share your private key. Anyone with this key can impersonate
              you and access everything in your account.
            </span>
          </p>
        ) : null}
      </div>

      <OnboardingFooter>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-next"
          disabled={backupNextDisabled()}
          onClick={onNext}
          type="button"
        >
          Next
        </Button>

        {mode === "raw" && loadError ? (
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
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
