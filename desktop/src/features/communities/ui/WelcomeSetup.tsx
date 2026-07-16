import * as React from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy } from "lucide-react";

import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { pubkeyToNpub } from "@/shared/lib/nostrUtils";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";

type WelcomeSetupPage = "welcome" | "join" | "invite";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialTransitionMode?: WelcomeTransitionMode;
};

const CREATE_COMMUNITY_URL = "https://buzz.xyz";
const LOCAL_DEV_RELAY_URLS = new Set([
  "ws://localhost:3000",
  "ws://127.0.0.1:3000",
]);

function isLocalDevRelayUrl(relayUrl: string) {
  return LOCAL_DEV_RELAY_URLS.has(relayUrl.trim().replace(/\/$/, ""));
}

export function WelcomeSetup({
  defaultRelayUrl,
  initialTransitionMode = "initial",
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

  const handleDefaultCommunity = React.useCallback(() => {
    communityOnboarding.start({
      source: "first-community",
      relayUrl: defaultRelayUrl,
    });
  }, [communityOnboarding, defaultRelayUrl]);

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
      className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground"
      data-system-color-scheme={systemColorScheme}
    >
      <StartupWindowDragRegion />
      <div className="relative flex w-full max-w-[500px] flex-col items-center text-center">
        {page === "welcome" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            effect={welcomeEffect}
            transitionKey={`welcome-${welcomeEffect}-${transitionDirection}`}
          >
            <img
              alt="Buzz"
              className="h-14 w-14 rounded-xl shadow-xs"
              src="/app-icon@2x.png"
              srcSet="/app-icon@2x.png 1x, /app-icon@3x.png 2x"
            />
            <h1 className="mt-6 text-3xl font-semibold tracking-tight">
              Welcome to Buzz
            </h1>
            <p className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground">
              Choose how you want to get started.
            </p>
            <div className="mt-8 flex w-full flex-col gap-3">
              {isLocalDevRelayUrl(defaultRelayUrl) ? null : (
                <Button
                  className="h-10 w-full"
                  onClick={handleDefaultCommunity}
                  type="button"
                >
                  Join default community
                </Button>
              )}
              <Button
                className="h-10 w-full"
                onClick={() => showPage("join")}
                type="button"
              >
                Join a community
              </Button>
              <Button
                className="h-10 w-full"
                onClick={() => showPage("invite")}
                type="button"
                variant="secondary"
              >
                I have an invite link
              </Button>
              <Button
                className="h-10 w-full"
                onClick={() => void openUrl(CREATE_COMMUNITY_URL)}
                type="button"
                variant="ghost"
              >
                Create a community
              </Button>
            </div>
          </OnboardingSlideTransition>
        ) : page === "join" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`join-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                Join a community
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Send your public key to a community owner. Keep Buzz open; once
                they add you, their invite link will continue setup here.
              </p>
              <div className="mt-8 space-y-2 text-left">
                <p className="text-xs font-medium text-muted-foreground">
                  Your public key (npub)
                </p>
                <div className="flex items-center gap-2">
                  <code
                    className="min-w-0 flex-1 break-all rounded-xl border border-border/70 bg-muted/30 px-3 py-2.5 font-mono text-xs"
                    data-testid="welcome-join-npub"
                  >
                    {npub || "Loading…"}
                  </code>
                  <Button
                    aria-label="Copy npub"
                    disabled={!npub}
                    onClick={() => {
                      void navigator.clipboard.writeText(npub).then(() => {
                        setCopied(true);
                        window.setTimeout(() => setCopied(false), 1500);
                      });
                    }}
                    size="icon"
                    type="button"
                    variant="outline"
                  >
                    {copied ? <Check /> : <Copy />}
                  </Button>
                </div>
                {identityError ? (
                  <p className="text-sm text-destructive">{identityError}</p>
                ) : (
                  <p className="text-xs leading-5 text-muted-foreground">
                    This is safe to share. It does not reveal your private key.
                  </p>
                )}
              </div>
              <Button
                className="mt-8 h-10 w-full"
                onClick={() => showPage("welcome")}
                type="button"
                variant="ghost"
              >
                Back
              </Button>
            </div>
          </OnboardingSlideTransition>
        ) : (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`invite-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                I have an invite link
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Keep this page open, then click the invite link you received.
                Buzz will continue automatically. You can also paste it below.
              </p>
            </div>
            <div className="mt-8 w-full">
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
              />
            </div>
          </OnboardingSlideTransition>
        )}
      </div>
    </div>
  );
}
