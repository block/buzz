import { CircleSlash2, HardDriveDownload, ShieldCheck } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS } from "./onboardingCardStyles";

const GUIDANCE_ICON_CLASS = `flex size-10 shrink-0 items-center justify-center rounded-full ${ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS}`;

export function IdentityKeyIntroduction({
  direction,
  disabled,
  onCreate,
  onOpenHelp,
}: {
  direction: OnboardingTransitionDirection;
  disabled: boolean;
  onCreate: () => void;
  onOpenHelp: () => void;
}) {
  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-stretch"
      data-testid="onboarding-page-key-intro"
      direction={direction}
      transitionKey={`identity-key-introduction-${direction}`}
    >
      <div className="w-full shrink-0 text-left">
        <h1 className="text-title font-normal text-foreground">
          Create a private identity key
        </h1>
        <p className="mt-2 text-base leading-6 text-foreground/75">
          This key will be how you log into Buzz. You can use it across Buzz
          communities and other platforms.
        </p>
        <div className="mt-2">
          <IdentityKeyHelpDialog inline onOpen={onOpenHelp} />
        </div>
      </div>

      <div className="flex w-full flex-1 items-center py-10">
        <div className="w-full space-y-6" data-testid="onboarding-key-guidance">
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span className={GUIDANCE_ICON_CLASS}>
              <ShieldCheck aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              Stored securely on this device
            </p>
          </div>
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span className={GUIDANCE_ICON_CLASS}>
              <CircleSlash2 aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              Never share it—anyone with this key can sign in as you
            </p>
          </div>
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span className={GUIDANCE_ICON_CLASS}>
              <HardDriveDownload aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              Use a secure backup to recover your account
            </p>
          </div>
        </div>
      </div>

      <OnboardingFooter>
        <Button
          className={ONBOARDING_PRIMARY_CTA_CLASS}
          data-testid="onboarding-create-private-key"
          disabled={disabled}
          onClick={onCreate}
          type="button"
        >
          {disabled ? "Creating key…" : "Create my private key"}
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
