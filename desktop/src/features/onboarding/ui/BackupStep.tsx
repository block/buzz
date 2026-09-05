import {
  Check,
  CircleSlash2,
  Copy,
  Download,
  Eye,
  EyeOff,
  HardDriveDownload,
  Info,
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
import { Card } from "@/shared/ui/card";
import { SpoilerInline } from "@/shared/ui/markdown/SpoilerInline";
import { supportsSpoilerParticles } from "@/shared/ui/SpoilerParticles";
import { Spinner } from "@/shared/ui/spinner";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { OnboardingFooter } from "./OnboardingFooter";
import { useOnboardingPreviewCardLayout } from "./OnboardingPreviewShell";
import {
  ONBOARDING_PREVIEW_NEUTRAL_SURFACE_CLASS,
  ONBOARDING_PREVIEW_SECONDARY_CTA_CLASS,
} from "./onboardingPreviewCardStyles";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ONBOARDING_KEY_TEXT_CLASS } from "./NsecMaskedDisplay";

/**
 * How long the "Creating your identity key" loader holds the stage before the
 * finished state fades in. Purely perceptual — the key already exists; the
 * pause sells the creation moment.
 */
const INTRO_HOLD_MS = 1400;
const PREVIEW_KEY_TEXT = `nsec1${"q".repeat(58)}`;

/**
 * The creation moment should only be sold once per app session. Module-level
 * so remounts (e.g. navigating Back and returning to this step) skip the fake
 * hold and show the finished state instantly.
 */
let introPlayed = false;

const REVEAL_ANIMATION_CLASS =
  "animate-in fade-in duration-700 motion-reduce:animate-none";
const V3_KEY_GUIDANCE_ICON_CLASS = cn(
  "flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-foreground/80",
  ONBOARDING_PREVIEW_NEUTRAL_SURFACE_CLASS,
);

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
  onOpenIdentityKeyHelp?: () => void;
  onOpenPasswordBackup: () => void;
  onShowOptions: () => void;
  optionsExpanded: boolean;
  lockedBackupCreated?: boolean;
  previewMode?: boolean;
  returningFromSecurity: boolean;
  showPreviewBackupShortcut?: boolean;
  /** Apply the experimental V3 copy and controls inside the safe preview. */
  v3Presentation?: boolean;
};

/**
 * Onboarding identity-key step — shows the freshly created key, then opens a
 * dark backup-options state. Copy fetches the raw key only after an explicit
 * click; password backup opens the separate security flow. Neither method
 * blocks Next.
 */
export function BackupStep({
  direction,
  identityStorage,
  onNext,
  onOpenIdentityKeyHelp,
  onOpenPasswordBackup,
  onShowOptions,
  optionsExpanded,
  lockedBackupCreated = false,
  previewMode = false,
  returningFromSecurity,
  showPreviewBackupShortcut = true,
  v3Presentation = false,
}: BackupStepProps) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const reduceMotion = useReducedMotion() ?? false;
  const [created, setCreated] = React.useState(introPlayed || reduceMotion);
  const [copyState, setCopyState] = React.useState<
    "idle" | "copying" | "copied"
  >("idle");
  const [copyError, setCopyError] = React.useState<string | null>(null);
  const [nsec, setNsec] = React.useState<string | null>(null);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const canUseSpoilerParticles = React.useMemo(
    () => v3Presentation && supportsSpoilerParticles(),
    [v3Presentation],
  );
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
      const value = nsec ?? (previewMode ? PREVIEW_KEY_TEXT : await getNsec());
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
  }, [nsec, previewMode]);

  const toggleReveal = React.useCallback(async () => {
    if (isRevealed) {
      setIsRevealed(false);
      return;
    }
    setCopyError(null);
    try {
      // The raw key enters the DOM only after this explicit reveal action.
      const value = nsec ?? (previewMode ? PREVIEW_KEY_TEXT : await getNsec());
      if (cancelledRef.current) return;
      setNsec(value);
      setIsRevealed(true);
    } catch (err) {
      if (cancelledRef.current) return;
      setCopyError(
        err instanceof Error ? err.message : "Failed to retrieve private key.",
      );
    }
  }, [isRevealed, nsec, previewMode]);

  // Fixed-length decorative mask (nsec keys are 63 chars) so no key material
  // is fetched just to render the blurred row. Bullets are joined with a
  // zero-width space: WebKit won't line-break a run of U+2022 without an
  // explicit break opportunity, so the masked row would overflow otherwise.
  const maskedKey = React.useMemo(
    () => Array.from({ length: nsec?.length ?? 63 }, () => "•").join("\u200b"),
    [nsec],
  );
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
  const introStorageDescription =
    identityStorage === "system-keyring"
      ? "Buzz keeps your identity key in your system keychain."
      : identityStorage === "local-file"
        ? "Buzz keeps your identity key in a private file on this device because the system keychain wasn’t available."
        : "Your identity key is protected on this device.";

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
            "mx-auto flex w-full shrink-0 flex-col",
            cardLayout ? "max-w-[500px] text-left" : "max-w-140 text-center",
          )}
        >
          <h1 className="text-title font-normal text-foreground">
            {v3Presentation
              ? "Create a private identity key"
              : "Backup options"}
          </h1>
          {v3Presentation ? (
            <div className={cardLayout ? "mt-2" : "mt-5"}>
              <p className="text-sm leading-6 text-foreground/75">
                This key will be how you log into Buzz. You can use it across
                Buzz communities and other platforms.
              </p>
              <div className="mt-2">
                <IdentityKeyHelpDialog
                  inline
                  onPreviewOpen={onOpenIdentityKeyHelp}
                  previewMode={previewMode}
                  v3Presentation={v3Presentation}
                />
              </div>
            </div>
          ) : (
            <p className="mt-5 text-sm leading-6 text-foreground/75">
              Your identity key works like a password for your Buzz account.
              Keep a copy somewhere safe. You can create a backup file and lock
              it with a password you can remember.
            </p>
          )}
        </div>

        {v3Presentation ? (
          <div className="mx-auto flex w-full max-w-[500px] flex-1 items-center py-10">
            <div
              className="w-full space-y-6"
              data-testid="onboarding-preview-key-guidance"
            >
              <div className="flex min-h-14 items-center gap-4 text-left">
                <span className={V3_KEY_GUIDANCE_ICON_CLASS}>
                  <ShieldCheck className="h-5 w-5" aria-hidden="true" />
                </span>
                <p className="text-base leading-6 text-foreground">
                  Stored securely on this device
                </p>
              </div>
              <div className="flex min-h-14 items-center gap-4 text-left">
                <span className={V3_KEY_GUIDANCE_ICON_CLASS}>
                  <CircleSlash2 className="h-5 w-5" aria-hidden="true" />
                </span>
                <p className="text-base leading-6 text-foreground">
                  Never share it—anyone with this key can sign in as you
                </p>
              </div>
              <div className="flex min-h-14 items-center gap-4 text-left">
                <span className={V3_KEY_GUIDANCE_ICON_CLASS}>
                  <HardDriveDownload className="h-5 w-5" aria-hidden="true" />
                </span>
                <p className="text-base leading-6 text-foreground">
                  Use a secure backup to recover your account
                </p>
              </div>
            </div>
          </div>
        ) : (
          <div className="flex w-full max-w-260 flex-1 flex-col justify-center py-10">
            <div
              className="grid w-full grid-cols-1 gap-5 md:grid-cols-2 lg:grid-cols-3"
              data-testid="backup-options"
            >
              <div
                className={cn(
                  BACKUP_OPTION_CLASS,
                  "md:col-span-2 lg:col-span-1",
                )}
                data-testid="backup-option-panel"
              >
                <span className="text-lg font-medium">{storageTitle}</span>
                <span className="mt-3 block text-sm leading-6 text-foreground/65">
                  {storageDescription}
                </span>
              </div>

              <div
                className={BACKUP_OPTION_CLASS}
                data-testid="backup-option-panel"
              >
                <span className="text-lg font-medium">
                  Saved in your password manager
                </span>
                <span className="mt-3 block text-sm leading-6 text-foreground/65">
                  Copy your identity key, then save it in a password manager
                  like 1Password.
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
                className={BACKUP_OPTION_CLASS}
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
                  {lockedBackupCreated ? (
                    <Check className="h-5 w-5" aria-hidden="true" />
                  ) : (
                    <ShieldCheck className="h-5 w-5" aria-hidden="true" />
                  )}
                  {lockedBackupCreated
                    ? "Created locked backup"
                    : "Create locked backup"}
                </Button>
              </div>
            </div>

            {copyError ? (
              <p
                className="mt-4 text-center text-sm text-destructive"
                data-testid="backup-copy-error"
              >
                Could not retrieve your private key: {copyError}. You can
                continue and find it later in Settings &gt; Profile &gt;
                Identity.
              </p>
            ) : null}
          </div>
        )}

        {v3Presentation ? (
          <OnboardingFooter>
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-preview-private-key-next"
              onClick={onNext}
              type="button"
            >
              Create my private key
            </Button>
          </OnboardingFooter>
        ) : null}
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
          "flex w-full max-w-[500px] shrink-0 flex-col",
          cardLayout ? "text-left" : "text-center",
        )}
      >
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1
          className={`text-title font-normal text-foreground ${REVEAL_ANIMATION_CLASS}`}
          key={created ? "created" : "creating"}
        >
          {created && v3Presentation
            ? "Your private identity key"
            : created
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
            {v3Presentation ? (
              "Don’t share this key. Anyone who has it can access your account."
            ) : (
              <>
                {introStorageDescription} You can continue now, or{" "}
                <button
                  className="rounded-sm font-medium underline decoration-foreground/40 underline-offset-4 transition-colors hover:decoration-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                  data-testid="backup-options-link"
                  onClick={onShowOptions}
                  type="button"
                >
                  review backup options
                </button>{" "}
                for ways to restore your account.
              </>
            )}
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
            "flex w-full max-w-[1040px] flex-1 flex-col justify-center",
            cardLayout ? "py-6" : "py-10",
            REVEAL_ANIMATION_CLASS,
          )}
        >
          <div className="w-full">
            <Card
              className={cn("py-6", cardLayout ? "px-0" : "px-8")}
              variant="textured"
            >
              <div className="mx-auto w-full min-w-0 max-w-[832px]">
                <div className="flex items-center gap-4">
                  <div className="min-w-0 flex-1">
                    {canUseSpoilerParticles ? (
                      <div
                        className="buzz-onboarding-private-key-spoiler"
                        data-testid="backup-key-spoiler"
                      >
                        <SpoilerInline block revealOnHover>
                          <p
                            className={cn(
                              ONBOARDING_KEY_TEXT_CLASS,
                              v3Presentation && "buzz-onboarding-key-text-v3",
                              "select-text",
                            )}
                            data-testid="backup-key-value"
                          >
                            {PREVIEW_KEY_TEXT}
                          </p>
                        </SpoilerInline>
                      </div>
                    ) : (
                      <p
                        className={cn(
                          ONBOARDING_KEY_TEXT_CLASS,
                          v3Presentation && "buzz-onboarding-key-text-v3",
                          isRevealed && nsec
                            ? "select-text"
                            : "select-none blur-[4px]",
                        )}
                        data-testid="backup-key-value"
                      >
                        {isRevealed && nsec ? nsec : maskedKey}
                      </p>
                    )}
                  </div>
                  {canUseSpoilerParticles ? null : (
                    <Button
                      aria-label={
                        isRevealed ? "Hide private key" : "Reveal private key"
                      }
                      className="h-10 w-10 shrink-0 text-muted-foreground hover:text-foreground"
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
                  )}
                </div>

                {v3Presentation ? (
                  <div className="mt-4 flex justify-start">
                    <Button
                      className={cn(
                        ONBOARDING_SECONDARY_CTA_CLASS,
                        "-ml-5 gap-2 bg-transparent px-5 text-foreground/80 hover:bg-[#e2e2e2]/50 hover:text-foreground",
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
                ) : null}
              </div>
            </Card>

            {copyError ? (
              <p
                className="mt-4 text-center text-sm text-destructive"
                data-testid="backup-copy-error"
              >
                Could not retrieve your private key: {copyError}. You can
                continue and find it later in Settings &gt; Profile &gt;
                Identity.
              </p>
            ) : null}

            {!v3Presentation ? (
              <p className="mx-auto mt-5 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
                <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                <span>
                  Never share your private key. Anyone with this key can
                  impersonate you and access everything in your account.
                </span>
              </p>
            ) : null}
          </div>
        </div>
      )}

      <OnboardingFooter className={cn(REVEAL_ANIMATION_CLASS, "flex-nowrap")}>
        {v3Presentation && created ? (
          <Button
            className={cn(
              ONBOARDING_SECONDARY_CTA_CLASS,
              cardLayout && ONBOARDING_PREVIEW_SECONDARY_CTA_CLASS,
              "px-4",
            )}
            data-testid="backup-option-password"
            onClick={onOpenPasswordBackup}
            type="button"
            variant="ghost"
          >
            {lockedBackupCreated
              ? "Created locked backup"
              : "Create locked backup"}
          </Button>
        ) : null}
        {v3Presentation && created && showPreviewBackupShortcut ? (
          <Button
            className={cn(ONBOARDING_SECONDARY_CTA_CLASS, "gap-2")}
            data-testid="backup-options-link"
            onClick={onShowOptions}
            type="button"
            variant="ghost"
          >
            <Download className="h-4 w-4" aria-hidden="true" />
            Back up your identity
          </Button>
        ) : null}
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-next"
          disabled={!created || backupNextDisabled()}
          onClick={onNext}
          type="button"
        >
          {v3Presentation ? "I’ve saved my key" : "Next"}
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
