import * as React from "react";
import { createPortal } from "react-dom";

import { cn } from "@/shared/lib/cn";

const OnboardingFooterTargetContext = React.createContext<HTMLElement | null>(
  null,
);

/**
 * Renders the shared, bottom-docked CTA slot for an onboarding shell and
 * exposes it to descendant steps via context.
 *
 * The slot is a direct child of the shell — a sibling of the animated step
 * content — so CTAs portaled into it through `OnboardingFooter` escape
 * `OnboardingSlideTransition`'s transform. A transformed ancestor establishes a
 * containing block that would otherwise trap `position: fixed`, which is why
 * the CTAs can't simply live inside the step and use `fixed` themselves. The
 * slot stays inside the `.buzz-onboarding-neutral-theme` subtree so
 * `--buzz-welcome-chartreuse` and the theme color tokens still resolve for the
 * docked buttons.
 */
export function OnboardingFooterProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [target, setTarget] = React.useState<HTMLElement | null>(null);

  return (
    <OnboardingFooterTargetContext.Provider value={target}>
      {children}
      {/* Scrim: on pages taller than the viewport, content scrolls under the
          docked CTA. This bottom-anchored fade to the shell's bottom color
          (invisible on short pages and on the flat chartreuse landing) gives
          the CTA a floor to sit on instead of colliding with form fields. */}
      <div
        aria-hidden
        className="pointer-events-none fixed inset-x-0 bottom-0 z-10 h-36 bg-[linear-gradient(to_top,var(--buzz-onboarding-shell-bottom)_35%,transparent)]"
      />
      <div
        className="pointer-events-none fixed inset-x-0 bottom-5 z-20 flex justify-center px-4"
        data-testid="onboarding-footer-slot"
        ref={setTarget}
      />
    </OnboardingFooterTargetContext.Provider>
  );
}

/**
 * Portals a step's primary CTA group into the shell's bottom-docked footer slot
 * (see `OnboardingFooterProvider`). When no slot exists in context — screens
 * that render onboarding steps outside a provider, e.g. `KeyringLockedScreen` —
 * it falls back to rendering the CTA group inline so the buttons never vanish.
 */
export function OnboardingFooter({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  const target = React.useContext(OnboardingFooterTargetContext);
  const group = (
    <div
      className={cn(
        "flex w-full max-w-[500px] flex-col items-center gap-3",
        // The docked slot is click-through (`pointer-events-none`); re-enable
        // pointer events on the CTA group itself. Inline (no slot) needs no
        // override since it sits in normal flow.
        target && "pointer-events-auto",
        className,
      )}
    >
      {children}
    </div>
  );

  if (!target) {
    return group;
  }

  return createPortal(group, target);
}
