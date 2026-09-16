import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  ONBOARDING_SECURITY_PRIMARY_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "./OnboardingChrome";
import { useOnboardingCardLayout } from "./OnboardingCard";
import { OnboardingFooter } from "./OnboardingFooter";
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
}: DownloadKeyStepProps) {
  const reduceMotion = useReducedMotion() ?? false;
  const cardLayout = useOnboardingCardLayout();
  // Once the encrypted payload is saved, the creator advances to its guided
  // backup test while this surface keeps its own navigation.
  const hasCreated = session.created;
  const hasVerifiedBackup = session.verified;
  const hasSelectedBackup = session.test.stage === "password";
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
          "flex w-full shrink-0 flex-col",
          cardLayout ? "text-left" : "max-w-[500px] text-center",
        )}
        initial={headingEntrance}
        key={
          hasVerifiedBackup
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
          {hasVerifiedBackup
            ? "Your backup is verified"
            : hasSelectedBackup
              ? "Verify your backup"
              : hasCreated
                ? "Your backup is ready"
                : "Create a secure backup file"}
        </h1>
        <p
          className={cn(
            "leading-6 text-foreground/80",
            cardLayout ? "mt-2 text-base" : "mt-5 text-sm",
          )}
        >
          {hasVerifiedBackup
            ? "Your file and password can restore your identity."
            : hasSelectedBackup
              ? "Enter your password to make sure you can unlock this file."
              : hasCreated
                ? "Test your backup to make sure it works, or continue without testing."
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
            key={hasCreated ? "test-panel" : "password-panel"}
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
                createButtonClassName={
                  cardLayout
                    ? ONBOARDING_PRIMARY_CTA_CLASS
                    : ONBOARDING_SECURITY_PRIMARY_CTA_CLASS
                }
                createButtonPortal={primaryActionSlot}
                session={session}
                variant="spotlight"
                verifyButtonPortal={primaryActionSlot}
              />
            </div>
          </motion.div>
        </div>
      </div>

      <OnboardingFooter>
        <div
          className="flex justify-center"
          data-testid={
            hasCreated ? "onboarding-verify-slot" : "onboarding-create-slot"
          }
          ref={setPrimaryActionSlot}
        />
        {hasCreated ? (
          <Button
            className={
              hasVerifiedBackup
                ? cardLayout
                  ? ONBOARDING_PRIMARY_CTA_CLASS
                  : ONBOARDING_SECURITY_PRIMARY_CTA_CLASS
                : ONBOARDING_SECONDARY_CTA_CLASS
            }
            data-testid={
              hasVerifiedBackup ? "onboarding-finish" : "onboarding-skip"
            }
            onClick={onBack}
            type="button"
            variant="ghost"
          >
            {hasVerifiedBackup ? "Continue" : "Skip for now"}
          </Button>
        ) : null}
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
