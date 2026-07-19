import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy } from "lucide-react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { normalizeRelayUrl } from "@/features/communities/relayProbe";
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
import { Input } from "@/shared/ui/input";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { OnboardingChrome } from "@/features/onboarding/ui/OnboardingChrome";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

type WelcomeSetupPage = "welcome" | "join" | "invite";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialPage?: WelcomeSetupPage;
  initialTransitionMode?: WelcomeTransitionMode;
  onBack: () => void;
};

const CREATE_COMMUNITY_URL = "https://app.builderlab.xyz/signup?returnTo=/buzz";
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
              <div className="w-full max-w-[760px]">
                <h1
                  aria-label="Request access to community"
                  className="text-title font-normal"
                >
                  Request access to a community
                </h1>
                <p className="mx-auto mt-3 max-w-[430px] text-sm leading-6">
                  Ask the community host to send you an invite link or add you
                  directly using your public key.
                </p>
              </div>
              <div className="flex w-full flex-1 items-center justify-center pb-2 pt-6">
                <div className="w-full max-w-4xl space-y-16">
                  <section aria-labelledby="welcome-join-key-step">
                    <h2
                      className="mb-4 text-sm font-normal"
                      id="welcome-join-key-step"
                    >
                      Step 1: Share your public key
                    </h2>
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
                      className="mb-4 text-sm font-normal"
                      id="welcome-join-url-step"
                    >
                      Step 2: Paste in your community URL
                    </h2>
                    <Input
                      aria-label="Community URL"
                      autoCapitalize="none"
                      autoCorrect="off"
                      className="h-auto rounded-xl border-0 bg-white/50 px-8 py-7 text-center font-mono !text-4xl text-[color:var(--buzz-onboarding-backup-ink)] shadow-none placeholder:text-[color:var(--buzz-onboarding-backup-ink)] placeholder:opacity-10 focus-visible:ring-1 focus-visible:ring-[rgb(113_113_6_/_0.5)]"
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
                  className="h-10 w-44 rounded-full"
                  disabled={!relayUrl.trim()}
                  form="welcome-join-form"
                  type="submit"
                >
                  <span aria-hidden>Next</span>
                  <span className="sr-only">Join community</span>
                </Button>
                <Button
                  className="h-10 w-44 rounded-full bg-foreground/10 hover:bg-foreground/15"
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
