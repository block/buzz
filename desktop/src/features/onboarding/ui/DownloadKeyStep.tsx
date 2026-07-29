import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import {
  type EncryptedBackupSession,
  backupSessionToPasswordEntry,
  EncryptedBackupCreator,
} from "./EncryptedBackupCreator";

type DownloadKeyStepProps = {
  direction: OnboardingTransitionDirection;
  /**
   * Backup state owned by the parent flow so Back navigation (which unmounts
   * this step) doesn't discard the created Keycase, the entered password, or
   * the backup-test progress.
   */
  session: EncryptedBackupSession;
  onBack: () => void;
  onNext: () => void;
};

/**
 * Onboarding download step — the password-first encrypted key download
 * (Keycase) flow, promoted to its own page in the machine onboarding flow.
 * The raw key never enters this component: Rust builds the NIP-49 payload
 * locally and the native save dialog produces the user-owned file.
 */
export function DownloadKeyStep({
  direction,
  session,
  onBack,
  onNext,
}: DownloadKeyStepProps) {
  const reduceMotion = useReducedMotion() ?? false;
  // True once the encrypted payload exists — the create button (living in the
  // footer's primary slot) disappears with the form, so Next takes its place.
  const hasCreated = session.created;
  // True once the user has passed the backup test — until then Next stays
  // disabled and "Skip for now" remains the escape hatch.
  const hasVerified = session.verified;
  // Footer slot the creator portals its "Download" button into.
  const [createButtonSlot, setCreateButtonSlot] =
    React.useState<HTMLElement | null>(null);

  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-center"
      data-testid="onboarding-page-download"
      direction={direction}
      transitionKey={`download-${direction}`}
    >
      <div className="flex w-full max-w-[500px] shrink-0 flex-col text-center">
        {/* Plain string concat: cn()'s tailwind-merge misreads the custom
            text-title size token as conflicting with text-foreground. */}
        <h1 className="text-title font-normal text-foreground">
          {hasCreated
            ? "Now, test your backup"
            : "Backup your key with a password"}
        </h1>
        <p className="mt-5 text-sm leading-6 text-foreground/80">
          {hasCreated
            ? "Make sure your backup works: drop the file you just saved and unlock it with your password."
            : "Keep the downloaded file private — you need both it and your password to restore your identity. Save the backup password somewhere safe; Buzz cannot reset it if lost."}
        </p>
      </div>

      <div className="flex w-full max-w-[1040px] flex-1 flex-col justify-center py-10">
        <div className="w-full">
          <motion.div
            animate={{ opacity: 1, y: 0 }}
            initial={reduceMotion ? false : { opacity: 0, y: 12 }}
            transition={{ delay: 0.12, duration: 0.4, ease: "easeOut" }}
          >
            <Card className="w-full px-8 py-6" variant="textured">
              <div className="mx-auto w-full max-w-[832px]">
                <EncryptedBackupCreator
                  createButtonClassName={ONBOARDING_PRIMARY_CTA_CLASS}
                  createButtonPortal={createButtonSlot}
                  session={session}
                  variant="spotlight"
                />
              </div>
            </Card>
          </motion.div>
        </div>
      </div>

      <OnboardingFooter>
        {hasCreated ? (
          hasVerified ? (
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-next"
              onClick={onNext}
              type="button"
            >
              Next
            </Button>
          ) : (
            /* No disabled Next while the test is unfinished — skipping is
               the only way forward until verification succeeds. */
            <Button
              className="h-9 whitespace-nowrap rounded-full px-6 hover:bg-foreground/10"
              data-testid="onboarding-skip"
              onClick={onNext}
              type="button"
              variant="ghost"
            >
              Skip for now
            </Button>
          )
        ) : (
          /* Relative row keeps the Download CTA truly centered while Skip
             hangs off its right edge without shifting the center. */
          <div className="relative flex items-center justify-center">
            <div
              className="flex justify-center"
              data-testid="onboarding-create-slot"
              ref={setCreateButtonSlot}
            />
            <Button
              className="absolute left-full ml-3 h-9 animate-in whitespace-nowrap rounded-full px-6 fade-in fill-mode-backwards [animation-delay:1000ms] animation-duration-[500ms] hover:bg-foreground/10 motion-reduce:animate-none"
              data-testid="onboarding-skip"
              onClick={onNext}
              type="button"
              variant="ghost"
            >
              Skip for now
            </Button>
          </div>
        )}

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
          data-testid="onboarding-back"
          onClick={
            // From the test view, Back first returns to the password form;
            // only from the form does it leave the step entirely.
            hasCreated ? () => backupSessionToPasswordEntry(session) : onBack
          }
          type="button"
          variant="ghost"
        >
          Back
        </Button>

        {hasCreated ? null : (
          <p className="text-xs text-foreground/50">
            You can back up your key anytime in Settings &rarr; Profile &rarr;
            Identity.
          </p>
        )}
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
