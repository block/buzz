import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { EncryptedBackupCreator } from "./EncryptedBackupCreator";

/**
 * One half of the "[   ]" pair that clamps around the shell box once it
 * settles — the visual is the user working on the secure shell that
 * surrounds their key. Brackets close inward as they fade in.
 */
function ShellBracket({
  reduceMotion,
  side,
  visible,
}: {
  reduceMotion: boolean;
  side: "left" | "right";
  visible: boolean;
}) {
  return (
    <motion.div
      animate={
        visible
          ? { opacity: 1, x: 0 }
          : { opacity: 0, x: side === "left" ? -10 : 10 }
      }
      aria-hidden
      className={cn(
        // The bracket pair frames the h-16 (64px) box as a concentric 96px
        // square: each line sits 10px clear of the box on every side (plus
        // the 6px line itself), and the 2rem corner radius wraps the box's
        // rounded-2xl (16px) corners at a constant 16px offset. The vertical
        // line lives on the OUTER edge of the 32px-deep arm, so each bracket
        // pulls 16px back over the box (negative margin) to keep that line
        // 10px from the box edge.
        "h-24 w-8 shrink-0 border-black",
        side === "left"
          ? "-mr-4 rounded-l-[2rem] border-y-[6px] border-l-[6px]"
          : "-ml-4 rounded-r-[2rem] border-y-[6px] border-r-[6px]",
      )}
      initial={false}
      transition={
        reduceMotion ? { duration: 0 } : { duration: 0.35, ease: "easeOut" }
      }
    />
  );
}

type DownloadKeyStepProps = {
  direction: OnboardingTransitionDirection;
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
  onBack,
  onNext,
}: DownloadKeyStepProps) {
  const reduceMotion = useReducedMotion() ?? false;
  // True once the shell box has landed; gates the closing brackets.
  const [shellSettled, setShellSettled] = React.useState(false);
  // True once the encrypted payload exists — the create button (living in the
  // footer's primary slot) disappears with the form, so Next takes its place.
  const [hasCreated, setHasCreated] = React.useState(false);
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
          Backup your key
        </h1>
        <p className="mt-5 text-sm leading-6 text-foreground/80">
          Keep the downloaded file private — you need both it and your password
          to restore your identity. Save the password somewhere safe; Buzz
          cannot reset it if lost.
        </p>
      </div>

      <div className="flex w-full max-w-[1040px] flex-1 flex-col justify-center py-10">
        <div className="w-full">
          <div className="mb-6 flex items-center justify-center">
            <ShellBracket
              reduceMotion={reduceMotion}
              side="left"
              visible={shellSettled || reduceMotion}
            />
            <motion.div
              animate={{ opacity: 1, scale: 1 }}
              className="h-16 w-16 rounded-2xl bg-white"
              data-testid="backup-key-shell"
              initial={reduceMotion ? false : { opacity: 0, scale: 0.8 }}
              onAnimationComplete={() => setShellSettled(true)}
              transition={
                reduceMotion
                  ? { duration: 0 }
                  : { duration: 0.45, ease: [0.22, 1, 0.36, 1] }
              }
            />
            <ShellBracket
              reduceMotion={reduceMotion}
              side="right"
              visible={shellSettled || reduceMotion}
            />
          </div>
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
                  onCreated={() => setHasCreated(true)}
                  variant="spotlight"
                />
              </div>
            </Card>
          </motion.div>
        </div>
      </div>

      <OnboardingFooter>
        {hasCreated ? (
          <Button
            className={ONBOARDING_PRIMARY_CTA_CLASS}
            data-testid="onboarding-next"
            onClick={onNext}
            type="button"
          >
            Next
          </Button>
        ) : (
          <div
            className="flex justify-center"
            data-testid="onboarding-create-slot"
            ref={setCreateButtonSlot}
          />
        )}

        <Button
          className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
          data-testid="onboarding-back"
          onClick={onBack}
          type="button"
          variant="ghost"
        >
          Back
        </Button>

        <p className="text-xs text-foreground/50">
          You can back up your key anytime in Settings &rarr; Profile &rarr;
          Identity.
        </p>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
