import * as React from "react";

import { HostedCommunityOnboarding } from "@/features/communities/ui/HostedCommunityOnboarding";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import { OnboardingChrome } from "@/features/onboarding/ui/OnboardingChrome";
import {
  OnboardingFooter,
  OnboardingFooterProvider,
} from "@/features/onboarding/ui/OnboardingFooter";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

type WelcomeSetupPage = "welcome" | "join" | "member" | "owned";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  initialPage?: WelcomeSetupPage;
  initialTransitionMode?: WelcomeTransitionMode;
  onBack: () => void;
};

const COMMUNITY_OPTION_CARD_CLASS =
  "w-full max-w-[320px] items-center px-6 py-4 text-center text-sm font-normal leading-6 text-foreground [--buzz-card-textured-min-height:88px] transition-[filter] duration-150 ease-out hover:brightness-[0.98] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/35";
const COMMUNITY_ROLE_CARD_CLASS =
  "w-full max-w-[240px] items-center px-5 py-3 text-center text-sm font-normal text-foreground [--buzz-card-textured-min-height:64px] transition-[filter] duration-150 ease-out hover:brightness-[0.98] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/35";

export function WelcomeSetup({
  initialPage = "welcome",
  initialTransitionMode = "initial",
  onBack,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>(initialPage);
  const [transitionMode, setTransitionMode] =
    React.useState<WelcomeTransitionMode>(initialTransitionMode);
  const [showExistingChoices, setShowExistingChoices] = React.useState(false);
  const communityOnboarding = useCommunityOnboarding();
  const systemColorScheme = useSystemColorScheme();

  const showPage = React.useCallback((nextPage: WelcomeSetupPage) => {
    setTransitionMode(nextPage === "welcome" ? "backward" : "forward");
    setPage(nextPage);
  }, []);

  const showOwnedPage = React.useCallback(() => {
    setTransitionMode("forward");
    setPage("owned");
  }, []);

  const startConnection = React.useCallback(
    (relayUrl: string) => {
      communityOnboarding.start({
        source: "first-community",
        firstCommunityPage: page === "member" ? "member" : "join",
        relayUrl,
      });
    },
    [communityOnboarding, page],
  );

  const redeemInvite = React.useCallback(
    (relayUrl: string, code: string, policyReceipt?: string) => {
      communityOnboarding.start({
        source: "first-community",
        firstCommunityPage: page === "member" ? "member" : "join",
        relayUrl,
        inviteCode: code,
        policyReceipt,
      });
    },
    [communityOnboarding, page],
  );

  const transitionDirection =
    transitionMode === "backward" ? "backward" : "forward";
  const welcomeEffect =
    transitionMode === "backward" ? "line-slide" : "mask-reveal-up";

  return (
    <div
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex h-dvh items-start justify-center overflow-y-auto bg-background px-4 pb-36 pt-[106px] text-foreground"
      data-system-color-scheme={systemColorScheme}
    >
      <StartupWindowDragRegion />
      <OnboardingChrome current={5} />
      <OnboardingFooterProvider>
        <div className="relative flex min-h-0 w-full max-w-[920px] flex-1 flex-col items-center text-center">
          {page === "welcome" ? (
            <OnboardingSlideTransition
              className="flex h-full min-h-0 w-full flex-col items-center text-center"
              containerClassName="h-full min-h-0 [&>.buzz-onboarding-transition-line]:h-full"
              direction={transitionDirection}
              effect={welcomeEffect}
              transitionKey={`welcome-${welcomeEffect}-${transitionDirection}`}
            >
              <div className="w-full max-w-[760px]">
                <h1 className="text-title font-normal">
                  How would you like to get started?
                </h1>
                <p className="mt-3 text-sm leading-6 text-foreground/80">
                  Join a community, create your own, or reconnect one on this
                  device.
                </p>
              </div>
              <div className="flex w-full flex-1 flex-col items-center justify-center gap-5 py-8">
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button
                    data-testid="community-choice-join"
                    onClick={() => showPage("join")}
                    type="button"
                  >
                    <span>
                      <span className="block">Join a community</span>
                      <span className="mt-1 block text-xs text-foreground/65">
                        Use an invite link or community URL
                      </span>
                    </span>
                  </button>
                </Card>
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button
                    data-testid="community-choice-create"
                    onClick={() => showOwnedPage()}
                    type="button"
                  >
                    <span>
                      <span className="block">Create a community</span>
                      <span className="mt-1 block text-xs text-foreground/65">
                        Start something new
                      </span>
                    </span>
                  </button>
                </Card>
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button
                    aria-expanded={showExistingChoices}
                    onClick={() => setShowExistingChoices((shown) => !shown)}
                    type="button"
                  >
                    <span>
                      <span className="block">I already have a community</span>
                      <span className="mt-1 block text-xs text-foreground/65">
                        Reconnect on this device
                      </span>
                    </span>
                  </button>
                </Card>
                {showExistingChoices ? (
                  <div
                    className="flex w-full flex-wrap justify-center gap-4"
                    data-testid="existing-community-choices"
                  >
                    <Card
                      asChild
                      className={COMMUNITY_ROLE_CARD_CLASS}
                      variant="textured"
                    >
                      <button onClick={() => showOwnedPage()} type="button">
                        I own it
                      </button>
                    </Card>
                    <Card
                      asChild
                      className={COMMUNITY_ROLE_CARD_CLASS}
                      variant="textured"
                    >
                      <button onClick={() => showPage("member")} type="button">
                        I’m a member or admin
                      </button>
                    </Card>
                  </div>
                ) : null}
              </div>
              <OnboardingFooter>
                <Button
                  className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
                  data-testid="welcome-setup-back"
                  onClick={onBack}
                  type="button"
                  variant="ghost"
                >
                  Back
                </Button>
              </OnboardingFooter>
            </OnboardingSlideTransition>
          ) : page === "owned" ? (
            <OnboardingSlideTransition
              className="flex w-full flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`owned-${transitionDirection}`}
            >
              <HostedCommunityOnboarding onBack={() => showPage("welcome")} />
            </OnboardingSlideTransition>
          ) : (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-15.625rem)] w-full flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`${page}-${transitionDirection}`}
            >
              <div className="w-full max-w-[620px]">
                <h1 className="text-title font-normal">
                  {page === "member"
                    ? "Reconnect to your community"
                    : "Join a community"}
                </h1>
                <p className="mt-3 text-sm leading-6 text-foreground/80">
                  {page === "member"
                    ? "Enter the community URL or an invite link. Your role will be restored when you connect."
                    : "Enter the invite link or community URL you received."}
                </p>
              </div>
              <div className="flex w-full flex-1 items-center justify-center">
                <InviteRedeemForm
                  error={null}
                  isRedeeming={false}
                  onCancel={() => showPage("welcome")}
                  onConnect={startConnection}
                  onRedeem={redeemInvite}
                  placeholder="Invite link or community URL"
                  variant="onboarding-spotlight"
                />
              </div>
            </OnboardingSlideTransition>
          )}
        </div>
      </OnboardingFooterProvider>
    </div>
  );
}
