import * as React from "react";
import { Check, Copy } from "lucide-react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { normalizeRelayUrl } from "@/features/communities/relayProbe";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import {
  ONBOARDING_KEY_ROW_CLASS,
  ONBOARDING_KEY_TEXT_CLASS,
} from "@/features/onboarding/ui/NsecMaskedDisplay";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import {
  OnboardingFooter,
  OnboardingFooterProvider,
} from "@/features/onboarding/ui/OnboardingFooter";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { pubkeyToNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import {
  ONBOARDING_PRIMARY_CTA_CLASS,
  OnboardingChrome,
} from "@/features/onboarding/ui/OnboardingChrome";
import { HostedCommunityOnboarding } from "@/features/communities/ui/HostedCommunityOnboarding";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type WelcomeSetupPage = "welcome" | "join" | "invite" | "owned";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  initialPage?: WelcomeSetupPage;
  initialTransitionMode?: WelcomeTransitionMode;
  onBack: () => void;
};

const COMMUNITY_OPTION_CARD_CLASS =
  "w-full max-w-[320px] items-center px-6 py-4 text-center text-sm font-normal leading-6 text-foreground [--buzz-card-textured-min-height:88px] transition-[filter] duration-150 ease-out hover:brightness-[0.98] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/35";
const WIDE_TEXTURE_CARD_CLASS =
  "relative left-1/2 w-[min(calc(100%+10rem),calc(100vw-2rem))] max-w-[1040px] -translate-x-1/2 px-8 py-6 [--buzz-card-textured-min-height:192px]";
const WIDE_TEXTURE_CONTENT_CLASS = "mx-auto w-full max-w-[840px]";
const HORIZONTAL_INPUT_OVERFLOW_FADE = {
  WebkitMaskImage:
    "linear-gradient(to right, transparent, black 2rem, black calc(100% - 2rem), transparent)",
  maskImage:
    "linear-gradient(to right, transparent, black 2rem, black calc(100% - 2rem), transparent)",
};

export function WelcomeSetup({
  initialPage = "welcome",
  initialTransitionMode = "initial",
  onBack,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>(initialPage);
  const [transitionMode, setTransitionMode] =
    React.useState<WelcomeTransitionMode>(initialTransitionMode);
  const [npub, setNpub] = React.useState("");
  const [identityError, setIdentityError] = React.useState<string | null>(null);
  const [relayUrl, setRelayUrl] = React.useState("");
  const [relayUrlError, setRelayUrlError] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState(false);
  const communityOnboarding = useCommunityOnboarding();
  const systemColorScheme = useSystemColorScheme();

  React.useEffect(() => {
    if (page !== "join" || npub || identityError) return;
    void getIdentity()
      .then((identity) => setNpub(pubkeyToNpub(identity.pubkey)))
      .catch((error: unknown) =>
        setIdentityError(
          error instanceof Error
            ? error.message
            : "Could not load your public key.",
        ),
      );
  }, [identityError, npub, page]);

  const showPage = React.useCallback((nextPage: WelcomeSetupPage) => {
    if (nextPage === "join") setIdentityError(null);
    setTransitionMode(nextPage === "welcome" ? "backward" : "forward");
    setPage(nextPage);
  }, []);

  const handleJoin = React.useCallback(
    (event: React.FormEvent) => {
      event.preventDefault();
      const normalizedRelayUrl = normalizeRelayUrl(relayUrl);
      if (!normalizedRelayUrl) {
        setRelayUrlError("Enter a valid community URL.");
        return;
      }

      setRelayUrlError(null);
      communityOnboarding.start({
        source: "first-community",
        relayUrl: normalizedRelayUrl,
      });
    },
    [communityOnboarding, relayUrl],
  );

  const handleInviteRedeem = React.useCallback(
    (relayWsUrl: string, code: string, policyReceipt?: string) => {
      communityOnboarding.start({
        source: "first-community",
        relayUrl: relayWsUrl,
        inviteCode: code,
        policyReceipt,
      });
    },
    [communityOnboarding],
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
                  Join or create a community
                </h1>
                <p className="mt-3 text-sm leading-6 text-foreground/80">
                  Choose how you’d like to get started. If you have an invite
                  link, you can open it directly to continue setup.
                </p>
              </div>
              <div className="flex w-full flex-1 translate-y-16 flex-col items-center justify-center gap-20 py-8">
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button onClick={() => showPage("join")} type="button">
                    Add me to a community
                  </button>
                </Card>
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button onClick={() => showPage("invite")} type="button">
                    I have an invite link
                  </button>
                </Card>
                <Card
                  asChild
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  variant="textured"
                >
                  <button onClick={() => showPage("owned")} type="button">
                    <span className="max-w-52">
                      Create or connect to my own community
                    </span>
                  </button>
                </Card>
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
          ) : page === "join" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-15.625rem)] w-full flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`join-${transitionDirection}`}
            >
              <div className="w-full max-w-[760px]">
                <h1
                  aria-label="Request access to community"
                  className="text-title font-normal"
                >
                  Request access to a community
                </h1>
              </div>
              <div className="flex w-full flex-1 items-center justify-center">
                <div className="w-full max-w-[920px] space-y-16">
                  <section
                    aria-labelledby="welcome-join-key-step"
                    className="translate-y-12"
                  >
                    <h2
                      className="relative z-10 mb-8 text-sm font-normal"
                      id="welcome-join-key-step"
                    >
                      Step 1: Ask your community host to send you an invite link
                      or add you using your public key
                    </h2>
                    <Card
                      className={WIDE_TEXTURE_CARD_CLASS}
                      data-testid="welcome-join-npub-frame"
                      variant="textured"
                    >
                      <div className={WIDE_TEXTURE_CONTENT_CLASS}>
                        <div className={ONBOARDING_KEY_ROW_CLASS}>
                          <div className="min-w-0 flex-1">
                            <code
                              className={`${ONBOARDING_KEY_TEXT_CLASS} block`}
                              data-testid="welcome-join-npub"
                            >
                              {npub || "Loading…"}
                            </code>
                          </div>
                          <Button
                            aria-label="Copy npub"
                            className="h-10 w-10 shrink-0 text-[var(--buzz-onboarding-backup-ink)] hover:bg-transparent hover:text-foreground"
                            disabled={!npub}
                            onClick={() => {
                              void writeTextToClipboard(npub).then(() => {
                                setCopied(true);
                                window.setTimeout(() => setCopied(false), 1500);
                              });
                            }}
                            size="icon"
                            type="button"
                            variant="ghost"
                          >
                            {copied ? (
                              <Check
                                className="h-6 w-6 text-primary"
                                aria-hidden="true"
                              />
                            ) : (
                              <Copy className="h-6 w-6" aria-hidden="true" />
                            )}
                          </Button>
                        </div>
                      </div>
                    </Card>
                    {identityError ? (
                      <p className="mt-4 text-sm text-destructive">
                        {identityError}
                      </p>
                    ) : null}
                  </section>
                  <form
                    aria-labelledby="welcome-join-url-step"
                    className="mx-auto w-full"
                    id="welcome-join-form"
                    onSubmit={handleJoin}
                  >
                    <h2
                      className="relative z-10 mb-8 text-sm font-normal"
                      id="welcome-join-url-step"
                    >
                      Step 2: Paste in your community URL
                    </h2>
                    <Card
                      className={`${WIDE_TEXTURE_CARD_CLASS} [--buzz-card-textured-min-height:128px]`}
                      variant="textured"
                    >
                      <div
                        className={WIDE_TEXTURE_CONTENT_CLASS}
                        style={HORIZONTAL_INPUT_OVERFLOW_FADE}
                      >
                        <Input
                          aria-label="Community URL"
                          autoCapitalize="none"
                          autoCorrect="off"
                          className="h-auto rounded-none border-0 bg-transparent p-0 text-center font-mono !text-4xl text-[color:var(--buzz-onboarding-backup-ink)] shadow-none placeholder:text-[color:var(--buzz-onboarding-backup-ink)] placeholder:opacity-10 focus-visible:ring-0"
                          data-testid="welcome-join-community-url"
                          id="welcome-join-community-url"
                          onChange={(event) => {
                            setRelayUrl(event.target.value);
                            setRelayUrlError(null);
                          }}
                          placeholder="Enter community URL"
                          spellCheck={false}
                          type="url"
                          value={relayUrl}
                        />
                      </div>
                    </Card>
                    {relayUrlError ? (
                      <p className="mt-3 text-sm text-destructive">
                        {relayUrlError}
                      </p>
                    ) : null}
                  </form>
                </div>
              </div>
              <OnboardingFooter>
                <Button
                  className={ONBOARDING_PRIMARY_CTA_CLASS}
                  disabled={!relayUrl.trim()}
                  form="welcome-join-form"
                  type="submit"
                >
                  <span aria-hidden>Next</span>
                  <span className="sr-only">Join community</span>
                </Button>
                <Button
                  className="h-9 rounded-full bg-foreground/10 px-6 hover:bg-foreground/15"
                  onClick={() => showPage("welcome")}
                  type="button"
                  variant="ghost"
                >
                  Back
                </Button>
              </OnboardingFooter>
            </OnboardingSlideTransition>
          ) : (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-15.625rem)] w-full flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`invite-${transitionDirection}`}
            >
              <div className="w-full max-w-[500px]">
                <h1 className="text-title font-normal">
                  Enter your invite link
                </h1>
                <p className="mt-3 text-sm leading-6 text-foreground/80">
                  If you have an invite link for a community, paste it below to
                  continue setup.
                </p>
              </div>
              <div className="flex w-full flex-1 items-center justify-center">
                <InviteRedeemForm
                  error={null}
                  isRedeeming={false}
                  onCancel={() => showPage("welcome")}
                  onRedeem={handleInviteRedeem}
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
