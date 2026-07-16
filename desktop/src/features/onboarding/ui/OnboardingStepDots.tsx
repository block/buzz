import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";

/**
 * Positions in the first-launch flow: landing, identity/key, provider setup,
 * community choice, community profile, meet the team.
 */
export const TOTAL_ONBOARDING_PAGES = 6;

/**
 * Top-left paging indicator for the first-launch flow. The bee marks the
 * current page on a dot track; shown on every page after the landing screen.
 */
export function OnboardingStepDots({
  current,
  total = TOTAL_ONBOARDING_PAGES,
}: {
  current: number;
  total?: number;
}) {
  return (
    <div
      aria-hidden
      className="pointer-events-none fixed left-6 top-12 z-10 flex items-center gap-2.5 text-foreground"
      data-testid="onboarding-step-dots"
    >
      {Array.from({ length: total }, (_, i) => i + 1).map((position) =>
        position === current ? (
          <span className="block w-9" key={position}>
            <BuzzMark className="h-auto w-full" />
          </span>
        ) : (
          <span
            className="block h-[7px] w-[7px] rounded-full bg-foreground"
            key={position}
          />
        ),
      )}
    </div>
  );
}
