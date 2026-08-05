import { ZorroHat } from "@/shared/ui/zorro-logo/ZorroHat";

/**
 * Positions in the first-launch flow: landing, identity/key, harness setup,
 * default config, community choice, community profile, meet the team. Password
 * backup is an optional subview of identity/key, not another position.
 */
export const TOTAL_ONBOARDING_PAGES = 7;

/** Shared pill shape (38px tall) for every onboarding primary CTA. */
const ONBOARDING_CTA_SHAPE = "h-[2.375rem] rounded-full px-6";

/**
 * Primary CTA shared by the rose onboarding pages and dark security subview.
 * The shell supplies palette variables so the shape stays consistent while
 * each surface retains the appropriate contrast.
 */
export const ONBOARDING_PRIMARY_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} bg-[var(--buzz-onboarding-button)] text-[var(--buzz-onboarding-cta-label)] shadow-sm hover:bg-[var(--buzz-onboarding-button-hover)]`;

/** Inverted primary action used only on dark backup-security surfaces. */
export const ONBOARDING_SECURITY_PRIMARY_CTA_CLASS = `${ONBOARDING_CTA_SHAPE} bg-white text-black/80 hover:bg-white/90 hover:text-black`;

/**
 * Landing uses the same wine control treatment as the setup pages.
 */
export const ONBOARDING_LANDING_CTA_CLASS = ONBOARDING_PRIMARY_CTA_CLASS;

/** Shared quiet pill for secondary actions throughout onboarding. */
export const ONBOARDING_SECONDARY_CTA_CLASS =
  "h-9 rounded-full border border-[var(--buzz-onboarding-secondary-border)] bg-[var(--buzz-onboarding-secondary-bg)] px-6 text-[var(--buzz-onboarding-secondary-ink)] shadow-none hover:bg-[var(--buzz-onboarding-secondary-bg-hover)] hover:text-[var(--buzz-onboarding-secondary-ink)]";

/**
 * Icon-control styling for onboarding surfaces that sit on the textured card:
 * deep-red backup ink (`--buzz-onboarding-backup-ink`) with a plain
 * brighten-to-foreground hover (no hover pill, which would read as a floating
 * box on the texture). Used by the identity-help dialog close button and the
 * key-import reveal toggle.
 */
export const ONBOARDING_INK_ICON_CLASS =
  "text-[color:var(--buzz-onboarding-backup-ink)] hover:bg-transparent hover:text-foreground";

/** Icon controls on the dark noisy backup surfaces stay visually unboxed. */
export const ONBOARDING_SECURITY_ICON_CLASS =
  "text-muted-foreground hover:bg-transparent hover:text-foreground";

/**
 * Shared onboarding chrome shown on every page after the landing screen: a
 * static Zorro mark pinned to the top-left, and a centered pagination track that
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
      className="pointer-events-none fixed inset-x-0 top-12 z-10 flex items-center px-6 text-foreground"
    >
      <span className="block w-11" data-testid="onboarding-logo">
        <ZorroHat className="h-auto w-full" />
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
