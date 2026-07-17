import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy, Info } from "lucide-react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import {
  ONBOARDING_KEY_FRAME_CLASS,
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
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { OnboardingChrome } from "@/features/onboarding/ui/OnboardingChrome";

type WelcomeSetupPage = "welcome" | "join" | "invite";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialTransitionMode?: WelcomeTransitionMode;
  onBack: () => void;
};

const CREATE_COMMUNITY_URL = "https://buzz.xyz";
const LOCAL_DEV_RELAY_URLS = new Set([
  "ws://localhost:3000",
  "ws://127.0.0.1:3000",
]);
const COMMUNITY_OPTION_CARD_CLASS =
  "flex min-h-24 w-full max-w-[352px] items-center justify-center rounded-xl bg-white/75 px-6 py-4 text-center text-sm font-normal leading-6 text-foreground transition-colors duration-150 ease-out hover:bg-white/85 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-foreground/35";

function isLocalDevRelayUrl(relayUrl: string) {
  return LOCAL_DEV_RELAY_URLS.has(relayUrl.trim().replace(/\/$/, ""));
}

export function WelcomeSetup({
  defaultRelayUrl,
  initialTransitionMode = "initial",
  onBack,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>("welcome");
  const [transitionMode, setTransitionMode] =
    React.useState<WelcomeTransitionMode>(initialTransitionMode);
  const [npub, setNpub] = React.useState("");
  const [identityError, setIdentityError] = React.useState<string | null>(null);
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
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex min-h-dvh items-start justify-center overflow-y-auto bg-background px-4 pb-36 pt-[106px] text-foreground"
      data-system-color-scheme={systemColorScheme}
    >
      <StartupWindowDragRegion />
      <OnboardingChrome current={5} />
      <OnboardingFooterProvider>
        <div className="relative flex w-full max-w-4xl flex-col items-center text-center">
          {page === "welcome" ? (
            <OnboardingSlideTransition
              className="flex w-full flex-col items-center text-center"
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
              <div className="mt-28 flex w-full flex-col items-center gap-6">
                <button
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  onClick={() => showPage("join")}
                  type="button"
                >
                  Add me to a community
                </button>
                <button
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  onClick={() => showPage("invite")}
                  type="button"
                >
                  I have an invite link
                </button>
                <button
                  className={COMMUNITY_OPTION_CARD_CLASS}
                  onClick={() => void openUrl(CREATE_COMMUNITY_URL)}
                  type="button"
                >
                  <span className="max-w-44">I want to create a community</span>
                </button>
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
          ) : page === "join" ? (
            <OnboardingSlideTransition
              className="flex min-h-[calc(100dvh-15.625rem)] w-full flex-col items-center text-center"
              direction={transitionDirection}
              transitionKey={`join-${transitionDirection}`}
            >
              <div className="w-full max-w-[500px]">
                <h1 className="text-title font-normal">
                  Request access to community
                </h1>
                <p className="mt-3 text-sm leading-6 text-foreground/80">
                  Ask the community host to send you an invite link or add you
                  directly using your public key.
                </p>
              </div>
              <div className="flex w-full flex-1 items-center justify-center pb-4 pt-12">
                <div className="w-full max-w-4xl">
                  <div
                    className={ONBOARDING_KEY_FRAME_CLASS}
                    data-testid="welcome-join-npub-frame"
                  >
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
                        className="h-10 w-10 shrink-0 text-muted-foreground hover:text-foreground"
                        disabled={!npub}
                        onClick={() => {
                          void navigator.clipboard.writeText(npub).then(() => {
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
                  {identityError ? (
                    <p className="mt-4 text-sm text-destructive">
                      {identityError}
                    </p>
                  ) : (
                    <p className="mx-auto mt-6 flex max-w-[440px] items-start justify-center gap-1.5 text-center text-xs leading-5 text-[var(--buzz-onboarding-backup-ink)]">
                      <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                      <span>
                        This is safe to share. It does not reveal your private
                        key.
                      </span>
                    </p>
                  )}
                </div>
              </div>
              <OnboardingFooter>
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
              <div className="flex w-full flex-1 items-center justify-center pb-4 pt-12">
                <InviteRedeemForm
                  defaultRelayUrl={
                    isLocalDevRelayUrl(defaultRelayUrl)
                      ? undefined
                      : defaultRelayUrl
                  }
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
