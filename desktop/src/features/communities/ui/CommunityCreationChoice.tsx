import { ChevronRight, Cloud, Laptop } from "lucide-react";

import { Card } from "@/shared/ui/card";

const LOCAL_DESCRIPTION =
  "Private to this Mac. Only you can use it — nothing is shared online, no one can join, and other devices can't see it. Your agents still work.";
const HOSTED_DESCRIPTION =
  "Claim a Buzz address so your team can join from anywhere. Opens Builderlab.";

const ONBOARDING_OPTION_CLASS =
  "w-full max-w-[440px] px-6 py-5 text-left text-foreground transition-[filter] duration-150 ease-out hover:brightness-[0.98] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/35";
const DIALOG_OPTION_CLASS =
  "flex w-full items-center gap-3 rounded-xl border border-border/70 bg-muted/30 px-4 py-4 text-left transition-colors duration-150 ease-out hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring";

type CommunityCreationChoiceProps = {
  hasLocalCommunity?: boolean;
  onChooseHosted: () => void;
  onChooseLocal: () => void;
  showLocalOption: boolean;
  variant: "onboarding" | "dialog";
};

export function CommunityCreationChoice({
  hasLocalCommunity = false,
  onChooseHosted,
  onChooseLocal,
  showLocalOption,
  variant,
}: CommunityCreationChoiceProps) {
  const localLabel = hasLocalCommunity
    ? "Open your on-this-device community"
    : "Keep it on this device";

  if (variant === "onboarding") {
    return (
      <div className="flex w-full flex-col items-center gap-5">
        <Card asChild className={ONBOARDING_OPTION_CLASS} variant="textured">
          <button
            data-testid="community-create-hosted"
            onClick={onChooseHosted}
            type="button"
          >
            <span className="block text-sm font-medium">Host it online</span>
            <span className="mt-1 block text-xs leading-5 text-foreground/70">
              {HOSTED_DESCRIPTION}
            </span>
          </button>
        </Card>
        {showLocalOption ? (
          <Card asChild className={ONBOARDING_OPTION_CLASS} variant="textured">
            <button
              data-testid="community-choice-local"
              onClick={onChooseLocal}
              type="button"
            >
              <span className="block text-sm font-medium">{localLabel}</span>
              <span className="mt-1 block text-xs leading-5 text-foreground/70">
                {LOCAL_DESCRIPTION}
              </span>
            </button>
          </Card>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <button
        className={DIALOG_OPTION_CLASS}
        data-testid="community-create-hosted"
        onClick={onChooseHosted}
        type="button"
      >
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Cloud className="h-4 w-4" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-sm font-medium text-foreground">
            Host it online
          </span>
          <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
            {HOSTED_DESCRIPTION}
          </span>
        </span>
        <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground/60" />
      </button>
      {showLocalOption ? (
        <button
          className={DIALOG_OPTION_CLASS}
          data-testid="community-choice-local"
          onClick={onChooseLocal}
          type="button"
        >
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Laptop className="h-4 w-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block text-sm font-medium text-foreground">
              {localLabel}
            </span>
            <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
              {LOCAL_DESCRIPTION}
            </span>
          </span>
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground/60" />
        </button>
      ) : null}
    </div>
  );
}
