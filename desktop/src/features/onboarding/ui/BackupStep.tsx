import {
  Check,
  ChevronRight,
  Copy,
  FileLock2,
  ShieldCheck,
} from "lucide-react";
import { useReducedMotion } from "motion/react";
import * as React from "react";

import { getNsec } from "@/shared/api/tauriIdentity";
import type { IdentityStorage } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import { Spinner } from "@/shared/ui/spinner";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { useOnboardingCardLayout } from "./OnboardingCard";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ONBOARDING_KEY_TEXT_CLASS } from "./NsecMaskedDisplay";
import { ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS } from "./onboardingCardStyles";

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

const BACKUP_OPTION_CLASS =
  "flex min-h-48 w-full flex-col items-start justify-start px-6 py-5 text-left text-foreground";

/** Viewing the key never blocks onboarding — Next is always actionable. */
export function backupNextDisabled(): boolean {
  return false;
}

type BackupStepProps = {
  direction: OnboardingTransitionDirection;
  identityStorage?: IdentityStorage;
  onNext: () => void;
  onOpenPasswordBackup: () => void;
  optionsExpanded: boolean;
  returningFromSecurity: boolean;
};

/**
 * Onboarding identity-key step — shows the freshly created key, then opens a
 * dark backup-options state. Hovering obscures the visible key and presents an
 * explicit copy action; password backup opens the separate security flow.
 * Neither method blocks Next.
 */
export function BackupStep({
  direction,
  identityStorage,
  onNext,
  onOpenPasswordBackup,
  optionsExpanded,
  returningFromSecurity,
}: BackupStepProps) {
  const reduceMotion = useReducedMotion() ?? false;
  const cardLayout = useOnboardingCardLayout();
  const [created, setCreated] = React.useState(introPlayed || reduceMotion);
  const [copyState, setCopyState] = React.useState<
    "idle" | "copying" | "copied"
  >("idle");
  const [copyError, setCopyError] = React.useState<string | null>(null);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const cancelledRef = React.useRef(false);
  const copiedTimerRef = React.useRef<number | null>(null);

  React.useEffect(() => {
    if (introPlayed) return;
    if (reduceMotion) {
      introPlayed = true;
      setCreated(true);
      return;
    }
    const timer = window.setTimeout(() => {
      introPlayed = true;
      setCreated(true);
    }, INTRO_HOLD_MS);
    return () => window.clearTimeout(timer);
  }, [reduceMotion]);

  React.useEffect(() => {
    cancelledRef.current = false;
    return () => {
      // Back-during-fetch: cancel any in-flight setState calls.
      cancelledRef.current = true;
      if (copiedTimerRef.current !== null)
        window.clearTimeout(copiedTimerRef.current);
    };
  }, []);

  React.useEffect(() => {
    void getNsec()
      .then((value) => {
        if (!cancelledRef.current) setNsec(value);
      })
      .catch((err: unknown) => {
        if (cancelledRef.current) return;
        setCopyError(
          err instanceof Error
            ? err.message
            : "Failed to retrieve private key.",
        );
      });
  }, []);

  const copyKeyToClipboard = React.useCallback(async () => {
    setCopyState("copying");
    setCopyError(null);
    try {
      const value = nsec ?? (await getNsec());
      if (!nsec && !cancelledRef.current) setNsec(value);
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

  const storageDescription =
    identityStorage === "system-keyring"
      ? "Buzz keeps your identity key in your system keychain. Your computer may ask for your password when Buzz needs to read the key."
      : identityStorage === "local-file"
        ? "Your system keychain wasn’t available, so Buzz keeps your identity key in a private file on this device."
        : "Buzz keeps your identity key protected on this device. Make a separate backup in case you lose access.";
  const storageTitle =
    identityStorage === "system-keyring"
      ? "Protected by your system keychain"
      : identityStorage === "local-file"
        ? "Stored in private device storage"
        : "Protected in private device storage";
  if (optionsExpanded) {
    return (
      <OnboardingSlideTransition
        className={cn(
          "flex min-h-0 w-full flex-col",
          cardLayout ? "items-stretch" : "items-center",
        )}
        data-testid="onboarding-page-backup-options"
        direction={direction}
        transitionKey={`backup-options-${direction}`}
      >
        <div
          className={cn(
            "flex w-full shrink-0 flex-col",
            cardLayout ? "text-left" : "max-w-140 text-center",
          )}
        >
          <h1 className="text-title font-normal text-foreground">
            Backup options
          </h1>
          <p
            className={cn(
              "leading-6 text-foreground/75",
              cardLayout ? "mt-2 text-base" : "mt-5 text-sm",
            )}
          >
            Your identity key works like a password for your Buzz account. Keep
            a copy somewhere safe. You can create a backup file and lock it with
            a password you can remember.
          </p>
        </div>

        <div
          className={cn(
            "flex w-full flex-1 flex-col justify-center",
            cardLayout ? "py-6" : "max-w-260 py-10",
          )}
        >
          <div
            className={cn(
              "grid w-full grid-cols-1",
              cardLayout ? "gap-2" : "gap-5 md:grid-cols-2 lg:grid-cols-3",
            )}
            data-testid="backup-options"
          >
            <div
              className={cn(
                BACKUP_OPTION_CLASS,
                cardLayout
                  ? `min-h-0 rounded-xl ${ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS}`
                  : "md:col-span-2 lg:col-span-1",
              )}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">{storageTitle}</span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                {storageDescription}
              </span>
            </div>

            <div
              className={cn(
                BACKUP_OPTION_CLASS,
                cardLayout &&
                  `min-h-0 rounded-xl ${ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS}`,
              )}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">
                Saved in your password manager
              </span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                Copy your identity key, then save it in a password manager like
                1Password.
              </span>
              <Button
                className={cn(
                  ONBOARDING_SECONDARY_CTA_CLASS,
                  "mt-5 w-fit gap-2 px-5",
                )}
                data-testid="backup-copy-key"
                disabled={copyState === "copying"}
                onClick={() => void copyKeyToClipboard()}
                type="button"
                variant="ghost"
              >
                {copyState === "copying" ? (
                  <Spinner className="h-4 w-4 border-2" />
                ) : copyState === "copied" ? (
                  <Check className="h-4 w-4" aria-hidden="true" />
                ) : (
                  <Copy className="h-4 w-4" aria-hidden="true" />
                )}
                {copyState === "copying"
                  ? "Copying…"
                  : copyState === "copied"
                    ? "Copied to clipboard"
                    : "Copy to clipboard"}
              </Button>
            </div>

            <div
              className={cn(
                BACKUP_OPTION_CLASS,
                cardLayout &&
                  `min-h-0 rounded-xl ${ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS}`,
              )}
              data-testid="backup-option-panel"
            >
              <span className="text-lg font-medium">
                Locked in a backup file
              </span>
              <span className="mt-3 block text-sm leading-6 text-foreground/65">
                Create a backup file and choose a password you can remember.
                You’ll need both to restore your account.
              </span>
              <Button
                className={cn(
                  ONBOARDING_SECONDARY_CTA_CLASS,
                  "mt-5 w-fit gap-2 px-5",
                )}
                data-testid="backup-option-password"
                onClick={onOpenPasswordBackup}
                type="button"
                variant="ghost"
              >
                <ShieldCheck className="h-5 w-5" aria-hidden="true" />
                Create locked backup
              </Button>
            </div>
          </div>

          {copyError ? (
            <p
              className="mt-4 text-center text-sm text-destructive"
              data-testid="backup-copy-error"
            >
              Could not retrieve your private key: {copyError}. You can continue
              and find it later in Settings &gt; Profile &gt; Identity.
            </p>
          ) : null}
        </div>
      </OnboardingSlideTransition>
    );
  }

  return (
    <OnboardingSlideTransition
      className={cn(
        "flex min-h-0 w-full flex-col",
        cardLayout ? "items-stretch" : "items-center",
      )}
      data-testid="onboarding-page-backup"
      direction={direction}
      transitionKey={`backup-${direction}-${returningFromSecurity ? "security" : "line"}`}
    >
      <div
        className={cn(
          "flex w-full shrink-0 flex-col",
          cardLayout ? "text-left" : "max-w-[500px] text-center",
        )}
      >
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1
          className={`text-title font-normal text-foreground ${REVEAL_ANIMATION_CLASS}`}
          key={created ? "created" : "creating"}
        >
          {created ? "Your private identity key" : "Creating your identity key"}
        </h1>
        {created ? (
          <p
            className={cn(
              cardLayout
                ? "mt-2 text-base leading-6 text-foreground/80"
                : "mt-5 text-sm leading-6 text-foreground/80",
              REVEAL_ANIMATION_CLASS,
            )}
          >
            Don’t share this key. Anyone who has it can access your account.
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
            "flex w-full max-w-[1040px] shrink-0 flex-col",
            cardLayout ? "mt-6" : "mt-10",
            REVEAL_ANIMATION_CLASS,
          )}
        >
          <div className="w-full">
            <div
              className="group/key relative flex h-[7.625rem] w-full items-center justify-center overflow-hidden rounded-xl border border-[#e5e5e5] bg-[#f5f5f5] px-4 py-6"
              data-testid="backup-key-well"
            >
              <p
                className={cn(
                  ONBOARDING_KEY_TEXT_CLASS,
                  "buzz-onboarding-key-text-v3 select-text break-all group-hover/key:select-none group-hover/key:blur-[4px] group-focus-within/key:select-none group-focus-within/key:blur-[4px]",
                )}
                data-testid="backup-key-value"
              >
                {nsec}
              </p>
              <div
                aria-hidden
                className="pointer-events-none absolute inset-px rounded-[11px] bg-white/60 opacity-0 transition-opacity duration-150 ease-out group-hover/key:opacity-100 group-focus-within/key:opacity-100 motion-reduce:transition-none"
              />
              <Button
                className="pointer-events-none absolute left-1/2 top-1/2 z-10 h-8 -translate-x-1/2 -translate-y-1/2 gap-2 rounded-full bg-foreground px-4 text-sm text-background opacity-0 shadow-none transition-opacity duration-150 ease-out group-hover/key:pointer-events-auto group-hover/key:opacity-100 group-focus-within/key:pointer-events-auto group-focus-within/key:opacity-100 hover:bg-foreground/90 hover:text-background motion-reduce:transition-none"
                data-testid="backup-copy-key"
                disabled={copyState === "copying"}
                onClick={() => void copyKeyToClipboard()}
                type="button"
                variant="ghost"
              >
                {copyState === "copying" ? (
                  <Spinner className="h-4 w-4 border-2" />
                ) : copyState === "copied" ? (
                  <Check aria-hidden className="h-4 w-4" />
                ) : (
                  <Copy aria-hidden className="h-4 w-4" />
                )}
                {copyState === "copying"
                  ? "Copying…"
                  : copyState === "copied"
                    ? "Copied to clipboard"
                    : "Copy to clipboard"}
              </Button>
            </div>

            <Button
              className="mt-2 h-12 w-full justify-between rounded-xl bg-transparent px-3 py-0 text-base font-normal text-foreground shadow-none transition-colors duration-150 ease-out hover:bg-foreground/[0.04] hover:text-foreground motion-reduce:transition-none"
              data-testid="backup-option-password"
              disabled={!created}
              onClick={onOpenPasswordBackup}
              type="button"
              variant="ghost"
            >
              <span className="flex min-w-0 items-center gap-3">
                <FileLock2 aria-hidden className="size-6" />
                <span>Create locked backup</span>
              </span>
              <ChevronRight aria-hidden className="size-5 text-foreground/70" />
            </Button>

            {copyError ? (
              <p
                className={cn(
                  "mt-4 text-sm text-destructive",
                  cardLayout ? "text-left" : "text-center",
                )}
                data-testid="backup-copy-error"
              >
                Could not retrieve your private key: {copyError}. You can
                continue and find it later in Settings &gt; Profile &gt;
                Identity.
              </p>
            ) : null}
          </div>
        </div>
      )}

      <OnboardingFooter className={REVEAL_ANIMATION_CLASS}>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-next"
          disabled={!created || backupNextDisabled()}
          onClick={onNext}
          type="button"
        >
          I’ve saved my key
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
