import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECURITY_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import { useOnboardingPreviewCardLayout } from "./OnboardingPreviewShell";
import { ONBOARDING_PREVIEW_SECONDARY_CTA_CLASS } from "./onboardingPreviewCardStyles";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import {
  type EncryptedBackupSession,
  EncryptedBackupCreator,
} from "./EncryptedBackupCreator";

type DownloadKeyStepProps = {
  direction: OnboardingTransitionDirection;
  /** Backup state owned by the parent flow across the creation and test views. */
  session: EncryptedBackupSession;
  onBack: () => void;
  /** Use in-memory backup operations inside the dev-only workshop preview. */
  previewMode?: boolean;
  /** Apply the experimental V3 wording inside the safe preview. */
  v3Presentation?: boolean;
};

/**
 * Password-backup security subview within the identity-key onboarding step.
 * The raw key never enters this component: Rust builds the NIP-49 payload
 * locally and the native save dialog produces the user-owned file.
 */
export function DownloadKeyStep({
  direction,
  session,
  onBack,
  previewMode = false,
  v3Presentation = false,
}: DownloadKeyStepProps) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const reduceMotion = useReducedMotion() ?? false;
  // Once the encrypted payload is saved, the creator advances to its guided
  // backup test while this surface keeps its own navigation.
  const hasCreated = session.created;
  const hasVerifiedBackup = session.verified;
  const hasSelectedBackup = session.test.stage === "password";
  const showPreviewSaved =
    v3Presentation && hasCreated && session.test.stage === "drop";
  const primaryActionClass = cardLayout
    ? ONBOARDING_PRIMARY_CTA_CLASS
    : ONBOARDING_SECURITY_PRIMARY_CTA_CLASS;
  const [primaryActionSlot, setPrimaryActionSlot] =
    React.useState<HTMLElement | null>(null);
  const headingEntrance = reduceMotion
    ? false
    : cardLayout
      ? { opacity: 0 }
      : { opacity: 0, y: 10 };
  const panelEntrance = reduceMotion
    ? false
    : cardLayout
      ? { opacity: 0 }
      : { opacity: 0, y: 12 };

  return (
    <OnboardingSlideTransition
      className={cn(
        "flex min-h-0 w-full flex-col",
        cardLayout ? "items-stretch" : "items-center",
      )}
      data-testid="onboarding-page-download"
      direction={direction}
      transitionKey={`download-${direction}`}
    >
      <motion.div
        animate={cardLayout ? { opacity: 1 } : { opacity: 1, y: 0 }}
        className={cn(
          "flex w-full max-w-[500px] shrink-0 flex-col",
          cardLayout ? "text-left" : "text-center",
        )}
        initial={headingEntrance}
        key={
          showPreviewSaved
            ? "saved-heading"
            : hasVerifiedBackup
              ? "success-heading"
              : hasCreated
                ? "test-heading"
                : "password-heading"
        }
        transition={{ duration: reduceMotion ? 0 : 0.3, ease: "easeOut" }}
      >
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1 className="text-title font-normal text-foreground">
          {showPreviewSaved
            ? "Your backup is ready"
            : hasVerifiedBackup
              ? "Your backup is verified"
              : hasSelectedBackup
                ? "That’s your backup file"
                : hasCreated
                  ? "Optionally, test your backup"
                  : "Create a secure backup file"}
        </h1>
        <p className="mt-5 text-sm leading-6 text-foreground/80">
          {showPreviewSaved
            ? "Test your backup to make sure it works, or download another copy."
            : hasVerifiedBackup
              ? "Your file and password can restore your identity."
              : hasSelectedBackup
                ? "Now enter your password to prove you can unlock it."
                : hasCreated
                  ? "Learn how your backup works. Drop the file you just saved and unlock it with your password."
                  : "This creates a password-protected file with your private key. Remember, Buzz can’t recover your key if you lose it."}
        </p>
      </motion.div>

      <div
        className={cn(
          "flex w-full max-w-[1040px] flex-col",
          cardLayout ? "py-8" : "flex-1 justify-center py-10",
        )}
      >
        <div className="w-full">
          <motion.div
            animate={cardLayout ? { opacity: 1 } : { opacity: 1, y: 0 }}
            initial={panelEntrance}
            key={
              showPreviewSaved
                ? "saved-panel"
                : hasCreated
                  ? "test-panel"
                  : "password-panel"
            }
            transition={{
              delay: reduceMotion ? 0 : 0.12,
              duration: reduceMotion ? 0 : 0.4,
              ease: "easeOut",
            }}
          >
            <div
              className={cn(
                "flex w-full max-w-140",
                cardLayout
                  ? "justify-start py-6"
                  : "mx-auto justify-center px-6 py-5",
              )}
              data-testid="backup-password-panel"
            >
              <EncryptedBackupCreator
                createButtonClassName={primaryActionClass}
                createButtonPortal={primaryActionSlot}
                previewMode={previewMode}
                session={session}
                variant="spotlight"
                v3Presentation={v3Presentation}
                verifyButtonPortal={primaryActionSlot}
              />
            </div>
          </motion.div>
        </div>
      </div>

      <OnboardingFooter>
        {showPreviewSaved ? (
          <Button
            className={primaryActionClass}
            data-testid="onboarding-preview-backup-done"
            onClick={onBack}
            type="button"
          >
            Continue
          </Button>
        ) : (
          <div
            className="flex justify-center"
            data-testid={
              hasCreated ? "onboarding-verify-slot" : "onboarding-create-slot"
            }
            ref={setPrimaryActionSlot}
          />
        )}
        {hasCreated && !showPreviewSaved ? (
          <Button
            className={
              hasVerifiedBackup
                ? primaryActionClass
                : cn(
                    ONBOARDING_SECONDARY_CTA_CLASS,
                    cardLayout && ONBOARDING_PREVIEW_SECONDARY_CTA_CLASS,
                  )
            }
            data-testid={
              hasVerifiedBackup ? "onboarding-finish" : "onboarding-skip"
            }
            onClick={onBack}
            type="button"
            variant="ghost"
          >
            {hasVerifiedBackup
              ? v3Presentation
                ? "Continue"
                : "Finish"
              : "Skip for now"}
          </Button>
        ) : null}
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
