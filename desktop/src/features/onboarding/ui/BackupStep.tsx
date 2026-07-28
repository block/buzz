import { ArrowLeft, Check, Copy, Eye, EyeOff, Info } from "lucide-react";
import * as React from "react";

import { getNsec } from "@/shared/api/tauriIdentity";
import { cn } from "@/shared/lib/cn";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import { Card } from "@/shared/ui/card";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { EncryptedBackupCreator } from "./EncryptedBackupCreator";
import { ONBOARDING_KEY_TEXT_CLASS } from "./NsecMaskedDisplay";

/**
 * How long the "Creating your identity key" loader holds the stage before the
 * finished state fades in. Purely perceptual — the key already exists; the
 * pause sells the creation moment.
 */
const INTRO_HOLD_MS = 1400;

/**
 * The creation moment should only be sold once per app session. Module-level
 * so remounts (e.g. navigating Back and returning to this step) skip the fake
 * hold and show the finished state instantly.
 */
let introPlayed = false;

const REVEAL_ANIMATION_CLASS =
  "animate-in fade-in duration-700 motion-reduce:animate-none";

/** Quicker fade for switching between the chooser and a backup method. */
const VIEW_ANIMATION_CLASS =
  "animate-in fade-in duration-300 motion-reduce:animate-none";

/**
 * The chooser offers both backup methods: copying happens in place, while the
 * encrypted download flow expands into its own view.
 */
type BackupView = "choose" | "download";

function BackToOptions({ onClick }: { onClick: () => void }) {
  return (
    <div className="mt-4 flex justify-center">
      <Button
        className="h-8 gap-1 text-sm text-muted-foreground hover:text-accent-foreground"
        data-testid="backup-back-to-options"
        onClick={onClick}
        size="sm"
        type="button"
        variant="ghost"
      >
        <ArrowLeft className="h-3.5 w-3.5" aria-hidden="true" />
        All backup options
      </Button>
    </div>
  );
}

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
 * Onboarding backup step — offers two ways to back up the new key without
 * blocking setup: a portable password-protected Keycase (recommended) or a
 * direct clipboard copy destined for a password manager. The raw key is
 * fetched only when the user explicitly clicks Copy, goes straight to the
 * clipboard, and is never rendered or held in state.
 */
export function BackupStep({ direction, onBack, onNext }: BackupStepProps) {
  const [created, setCreated] = React.useState(introPlayed);
  const [view, setView] = React.useState<BackupView>("choose");
  const [copyState, setCopyState] = React.useState<
    "idle" | "copying" | "copied"
  >("idle");
  const [copyError, setCopyError] = React.useState<string | null>(null);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const cancelledRef = React.useRef(false);
  const copiedTimerRef = React.useRef<number | null>(null);

  React.useEffect(() => {
    if (introPlayed) return;
    const timer = window.setTimeout(() => {
      introPlayed = true;
      setCreated(true);
    }, INTRO_HOLD_MS);
    return () => window.clearTimeout(timer);
  }, []);

  React.useEffect(() => {
    cancelledRef.current = false;
    return () => {
      // Back-during-fetch: cancel any in-flight setState calls and clear the
      // nsec from memory on unmount (backup step is only on the fresh-key path).
      cancelledRef.current = true;
      setNsec(null);
      if (copiedTimerRef.current !== null)
        window.clearTimeout(copiedTimerRef.current);
    };
  }, []);

  const copyKeyToClipboard = React.useCallback(async () => {
    setCopyState("copying");
    setCopyError(null);
    try {
      const value = nsec ?? (await getNsec());
      await writeTextToClipboard(value);
      if (cancelledRef.current) return;
      setCopyState("copied");
      if (copiedTimerRef.current !== null)
        window.clearTimeout(copiedTimerRef.current);
      copiedTimerRef.current = window.setTimeout(() => {
        if (!cancelledRef.current) setCopyState("idle");
      }, 2000);
    } catch (err) {
      if (cancelledRef.current) return;
      setCopyState("idle");
      setCopyError(
        err instanceof Error ? err.message : "Failed to retrieve private key.",
      );
    }
  }, [nsec]);

  const toggleReveal = React.useCallback(async () => {
    if (isRevealed) {
      setIsRevealed(false);
      return;
    }
    setCopyError(null);
    try {
      // The raw key enters the DOM only after this explicit reveal action.
      const value = nsec ?? (await getNsec());
      if (cancelledRef.current) return;
      setNsec(value);
      setIsRevealed(true);
    } catch (err) {
      if (cancelledRef.current) return;
      setCopyError(
        err instanceof Error ? err.message : "Failed to retrieve private key.",
      );
    }
  }, [isRevealed, nsec]);

  // Fixed-length decorative mask (nsec keys are 63 chars) so no key material
  // is fetched just to render the blurred row. Bullets are joined with a
  // zero-width space: WebKit won't line-break a run of U+2022 without an
  // explicit break opportunity, so the masked row would overflow otherwise.
  const maskedKey = React.useMemo(
    () => Array.from({ length: nsec?.length ?? 63 }, () => "•").join("\u200b"),
    [nsec],
  );

  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-center"
      data-testid="onboarding-page-backup"
      direction={direction}
      transitionKey={`backup-${direction}`}
    >
      <div className="flex w-full max-w-[500px] shrink-0 flex-col text-center">
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1
          className={`text-title font-normal text-foreground ${REVEAL_ANIMATION_CLASS}`}
          key={created ? "created" : "creating"}
        >
          {created
            ? "Your unique identity key has been created"
            : "Creating your identity key"}
        </h1>
        {created ? (
          <p
            className={cn(
              "mt-5 text-sm leading-6 text-foreground/80",
              REVEAL_ANIMATION_CLASS,
            )}
          >
            Your identity key will be saved to your keychain. Back it up
            somewhere safe so you can restore your account. Never share your
            key.
          </p>
        ) : null}
      </div>

      {!created ? (
        <div
          className="flex w-full flex-1 items-center justify-center py-10"
          data-testid="backup-intro-logo"
        >
          <FuzzyLogo
            ariaLabel="Creating your identity key"
            className="w-20! text-foreground"
            fuzz
            loop
            loopRestSeconds={0}
          />
        </div>
      ) : (
        <div
          className={cn(
            "flex w-full max-w-[1040px] flex-1 flex-col justify-center py-10",
            REVEAL_ANIMATION_CLASS,
          )}
        >
          {view === "choose" ? (
            <div className="w-full">
              <Card className="px-8 py-6" variant="textured">
                <div className="mx-auto flex w-full min-w-0 max-w-[832px] items-center gap-4">
                  <div className="min-w-0 flex-1">
                    <p
                      className={cn(
                        ONBOARDING_KEY_TEXT_CLASS,
                        isRevealed && nsec
                          ? "select-text"
                          : "select-none blur-[4px]",
                      )}
                      data-testid="backup-key-value"
                    >
                      {isRevealed && nsec ? nsec : maskedKey}
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-1.5">
                    <Button
                      aria-label={
                        isRevealed ? "Hide private key" : "Reveal private key"
                      }
                      className="h-10 w-10 text-muted-foreground hover:text-foreground"
                      data-testid="backup-key-reveal-toggle"
                      onClick={() => void toggleReveal()}
                      size="icon"
                      type="button"
                      variant="ghost"
                    >
                      {isRevealed ? (
                        <EyeOff className="h-6 w-6" aria-hidden="true" />
                      ) : (
                        <Eye className="h-6 w-6" aria-hidden="true" />
                      )}
                    </Button>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          aria-label="Copy private key"
                          className="h-10 w-10 text-muted-foreground hover:text-foreground"
                          data-testid="backup-copy-key"
                          disabled={copyState === "copying"}
                          onClick={() => void copyKeyToClipboard()}
                          size="icon"
                          type="button"
                          variant="ghost"
                        >
                          {copyState === "copying" ? (
                            <Spinner className="h-5 w-5 border-2" />
                          ) : copyState === "copied" ? (
                            <Check
                              className="h-6 w-6 text-primary"
                              aria-hidden="true"
                            />
                          ) : (
                            <Copy className="h-6 w-6" aria-hidden="true" />
                          )}
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent className="max-w-[240px] text-center">
                        Copy your key and save it somewhere safe — a password
                        manager is a great place for it.
                      </TooltipContent>
                    </Tooltip>
                  </div>
                </div>
                {copyError ? (
                  <p
                    className="mt-3 text-center text-sm text-destructive"
                    data-testid="backup-copy-error"
                  >
                    Could not retrieve your private key: {copyError}. You can
                    continue and find it later in Settings &gt; Profile &gt;
                    Identity.
                  </p>
                ) : null}
              </Card>

              <p className="mx-auto mt-6 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
                <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>
                  Never share your private key. Anyone with this key can
                  impersonate you and access everything in your account.
                </span>
              </p>
            </div>
          ) : (
            <div className={VIEW_ANIMATION_CLASS}>
              <Card className="w-full px-8 py-6" variant="textured">
                <div className="mx-auto w-full max-w-[832px]">
                  <div className="mb-5 space-y-2 text-center">
                    <h2 className="text-lg font-medium">Download your key</h2>
                    <p className="text-xs leading-5 text-muted-foreground">
                      Your key is encrypted with your password before it
                      downloads. Keep the file private — you need both it and
                      the password to restore your identity.
                    </p>
                  </div>
                  <EncryptedBackupCreator variant="spotlight" />
                </div>
              </Card>
              <BackToOptions onClick={() => setView("choose")} />
            </div>
          )}
        </div>
      )}

      {created ? (
        <OnboardingFooter className={REVEAL_ANIMATION_CLASS}>
          <Button
            className={ONBOARDING_PRIMARY_CTA_CLASS}
            data-testid="onboarding-next"
            disabled={backupNextDisabled()}
            onClick={onNext}
            type="button"
          >
            Next
          </Button>

          {view === "choose" ? (
            <Button
              className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
              data-testid="backup-option-download"
              onClick={() => setView("download")}
              type="button"
              variant="ghost"
            >
              Download my key
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
      ) : null}
    </OnboardingSlideTransition>
  );
}
