import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";

/**
 * Positions in the first-launch flow: landing, identity/key, harness setup,
 * default config, community choice, community profile, meet the team. Used as
 * the default pagination length when a flow doesn't pass an explicit total.
 */
export const TOTAL_ONBOARDING_PAGES = 7;

/** Shared pill shape (38px tall) for every onboarding primary CTA. */
const ONBOARDING_CTA_SHAPE = "h-[2.375rem] rounded-full px-6";

/**
 * Primary-CTA styling for the in-step onboarding pages: the shared pill with a
 * light-blue label (`--buzz-onboarding-cta-label`, available on any
 * `.buzz-onboarding-neutral-theme` subtree).
 */
export const ONBOARDING_PRIMARY_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} text-[var(--buzz-onboarding-cta-label)]`;

/**
 * Primary-CTA styling for the landing screen only: the shared pill with the
 * chartreuse label (`--buzz-welcome-chartreuse`). The blue label is reserved
 * for the CTAs inside the steps.
 */
export const ONBOARDING_LANDING_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} text-[var(--buzz-welcome-chartreuse)]`;

/**
 * Shared onboarding chrome shown on every page after the landing screen: a
 * static Buzz mark pinned to the top-left, and a centered pagination track that
 * sits above the page title. The active page reads as a longer bar; inactive
 * pages are dots.
 */
export function OnboardingChrome({
  current,
  total = TOTAL_ONBOARDING_PAGES,
}: {
  current: number;
  total?: number;
}) {
  return (
    <div
      aria-hidden
      className="pointer-events-none fixed inset-x-0 top-14 z-10 flex items-center px-6 text-foreground"
    >
      <span className="block w-11" data-testid="onboarding-logo">
        <BuzzMark className="h-auto w-full" />
      </span>
      <div
        className="absolute left-1/2 top-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center gap-2"
        data-testid="onboarding-step-dots"
      >
        {Array.from({ length: total }, (_, i) => i + 1).map((position) => (
          <span
            className={
              position === current
                ? "block h-1.5 w-7 rounded-full bg-foreground"
                : "block h-1.5 w-1.5 rounded-full bg-foreground/30"
            }
            key={position}
          />
        ))}
      </div>
    </div>
  );
}
