import { CircleSlash2, HardDriveDownload, ShieldCheck } from "lucide-react";

import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { IdentityKeyHelpDialog } from "./IdentityKeyHelpDialog";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS } from "./onboardingCardStyles";

const GUIDANCE_ICON_CLASS = `flex size-10 shrink-0 items-center justify-center rounded-full text-foreground ${ONBOARDING_CARD_NEUTRAL_SURFACE_CLASS}`;

export function IdentityKeyIntroduction({
  direction,
  disabled,
  error,
  onCreate,
  onOpenHelp,
}: {
  direction: OnboardingTransitionDirection;
  disabled: boolean;
  error?: string | null;
  onCreate: () => void;
  onOpenHelp: () => void;
}) {
  const { t } = useTranslation();
  return (
    <OnboardingSlideTransition
      className="flex min-h-0 w-full flex-col items-stretch"
      data-testid="onboarding-page-key-intro"
      direction={direction}
      transitionKey={`identity-key-introduction-${direction}`}
    >
      <div className="w-full shrink-0 text-left">
        <h1 className="text-title font-normal text-foreground">
          {t("onboarding.key-intro.title")}
        </h1>
        <p className="mt-2 text-base leading-6 text-foreground/75">
          {t("onboarding.key-intro.body")}
        </p>
        <div className="mt-2">
          <IdentityKeyHelpDialog inline onOpen={onOpenHelp} />
        </div>
        {error ? (
          <p
            className="mt-4 rounded-xl bg-destructive/10 px-4 py-3 text-sm text-destructive"
            data-testid="identity-key-create-error"
            role="alert"
          >
            {error}
          </p>
        ) : null}
      </div>

      <div className="flex w-full flex-1 items-center py-10">
        <div className="w-full space-y-6" data-testid="onboarding-key-guidance">
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span
              className={GUIDANCE_ICON_CLASS}
              data-testid="identity-key-guidance-icon"
            >
              <ShieldCheck aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              {t("onboarding.key-intro.guidance-stored")}
            </p>
          </div>
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span
              className={GUIDANCE_ICON_CLASS}
              data-testid="identity-key-guidance-icon"
            >
              <CircleSlash2 aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              {t("onboarding.key-intro.guidance-never-share")}
            </p>
          </div>
          <div className="flex min-h-14 items-center gap-4 text-left">
            <span
              className={GUIDANCE_ICON_CLASS}
              data-testid="identity-key-guidance-icon"
            >
              <HardDriveDownload aria-hidden className="size-5" />
            </span>
            <p className="text-base leading-6 text-foreground">
              {t("onboarding.key-intro.guidance-backup")}
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
          {disabled
            ? t("onboarding.key-intro.creating")
            : t("onboarding.key-intro.create-button")}
        </Button>
      </OnboardingFooter>
    </OnboardingSlideTransition>
  );
}
