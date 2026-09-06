import { RotateCcw, ShieldCheck } from "lucide-react";

import type { OnboardingPreviewVariant } from "../onboardingPreview";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import { SegmentedControl } from "@/shared/ui/segmented-control";

const PREVIEW_VARIANT_OPTIONS = [
  { label: "Onboarding today", value: "today" },
  { label: "V3", value: "v3" },
] as const;

export function OnboardingPreviewControls({
  chooseHarnessConnectionMethodFirst,
  harnessConnectionInOnboarding,
  onChooseHarnessConnectionMethodFirstChange,
  onHarnessConnectionInOnboardingChange,
  onRestart,
  onVariantChange,
  variant,
}: {
  chooseHarnessConnectionMethodFirst: boolean;
  harnessConnectionInOnboarding: boolean;
  onChooseHarnessConnectionMethodFirstChange: (enabled: boolean) => void;
  onHarnessConnectionInOnboardingChange: (included: boolean) => void;
  onRestart: () => void;
  onVariantChange: (variant: OnboardingPreviewVariant) => void;
  variant: OnboardingPreviewVariant;
}) {
  return (
    <aside
      className="buzz-onboarding-neutral-theme fixed right-4 top-4 z-[100] flex w-[320px] max-w-[calc(100vw-2rem)] flex-col gap-3 rounded-2xl border border-foreground/15 bg-background/95 px-4 py-3 text-left shadow-lg backdrop-blur"
      data-system-color-scheme="light"
      data-testid="onboarding-preview-banner"
    >
      <div className="flex items-center gap-3">
        <ShieldCheck className="h-5 w-5 shrink-0 text-primary" aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-foreground">
            Workshop preview
          </p>
          <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
            Changes in this workshop aren’t saved.
          </p>
        </div>
        <Button
          aria-label="Restart onboarding preview"
          className="h-8 w-8 shrink-0 text-foreground hover:text-foreground"
          data-testid="onboarding-preview-restart"
          onClick={onRestart}
          size="icon"
          type="button"
          variant="ghost"
        >
          <RotateCcw className="h-4 w-4" aria-hidden />
        </Button>
      </div>
      <SegmentedControl
        className="w-full"
        legend="Onboarding version"
        onValueChange={onVariantChange}
        optionTestIdPrefix="onboarding-preview-variant"
        options={PREVIEW_VARIANT_OPTIONS}
        size="wide"
        testId="onboarding-preview-variant"
        value={variant}
      />
      {variant === "v3" ? (
        <div className="space-y-2">
          <label
            className="flex cursor-pointer items-center gap-2 text-xs font-medium text-foreground"
            htmlFor="onboarding-preview-harness-placement"
          >
            <Checkbox
              checked={harnessConnectionInOnboarding}
              className="border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background"
              id="onboarding-preview-harness-placement"
              onCheckedChange={(checked) =>
                onHarnessConnectionInOnboardingChange(checked === true)
              }
            />
            <span>Harness connection in onboarding</span>
          </label>
          {harnessConnectionInOnboarding ? (
            <label
              className="flex cursor-pointer items-center gap-2 pl-6 text-xs font-medium text-foreground"
              htmlFor="onboarding-preview-harness-method-first"
            >
              <Checkbox
                checked={chooseHarnessConnectionMethodFirst}
                className="border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background"
                id="onboarding-preview-harness-method-first"
                onCheckedChange={(checked) =>
                  onChooseHarnessConnectionMethodFirstChange(checked === true)
                }
              />
              <span>Choose subscription or API first</span>
            </label>
          ) : null}
        </div>
      ) : null}
    </aside>
  );
}
