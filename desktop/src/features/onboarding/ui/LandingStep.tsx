import { BeeMark } from "../marketing/BeeMark";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";

/**
 * Onboarding landing (step 1) — the marketing-branded welcome surface.
 *
 * Chartreuse dot-grid background, ink bee mark + wordmark, a primary "Get
 * started" pill CTA, and a low-emphasis "Already have an account?" link that
 * routes to the existing key-import path.
 *
 * Typography uses the app's stock Inter for now — the marketing site's Cash
 * Sans is proprietary and not cleared for this OSS repo (see marketing.css).
 */
export type LandingStepActions = {
  getStarted: () => void;
  importExistingKey: () => void;
};

export function LandingStep({
  actions,
  direction,
}: {
  actions: LandingStepActions;
  direction: OnboardingTransitionDirection;
}) {
  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      direction={direction}
      transitionKey={`landing-${direction}`}
    >
      <div className="flex w-full max-w-[440px] flex-col items-center">
        <BeeMark className="buzz-marketing__mark buzz-marketing__mark-in mb-8" />

        <h1 className="text-4xl font-semibold tracking-tight">
          Welcome to Buzz
        </h1>
        <p className="mt-3 text-base leading-6 opacity-70">
          The secure messaging platform for teams and their agents
        </p>

        <button
          type="button"
          className="buzz-marketing__pill mt-8 text-base"
          data-testid="onboarding-landing-get-started"
          onClick={actions.getStarted}
        >
          Get started
        </button>

        <button
          type="button"
          className="buzz-marketing__link mt-5 text-sm"
          data-testid="onboarding-landing-import-key"
          onClick={actions.importExistingKey}
        >
          Sign in
        </button>
      </div>
    </OnboardingSlideTransition>
  );
}
