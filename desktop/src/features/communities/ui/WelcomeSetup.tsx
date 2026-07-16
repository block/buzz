import * as React from "react";
import { flushSync } from "react-dom";

import {
  getIdentity,
  importIdentity as tauriImportIdentity,
} from "@/shared/api/tauriIdentity";
import { claimInvite } from "@/shared/api/invites";
import { inviteErrorMessage } from "@/shared/api/inviteHelpers";
import { InviteRedeemForm } from "@/features/onboarding/ui/InviteRedeemForm";
import { NostrKeyImportForm } from "@/features/onboarding/ui/NostrKeyImportForm";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";

import type { Community } from "../types";
import { initFirstCommunity } from "../communityStorage";
import { CommunityEditForm } from "./CommunityEditForm";

export type WelcomeSetupPage =
  | "welcome"
  | "create-community"
  | "invite"
  | "nostr-key";

// Sub-page headings, also rendered by the first-run connecting gate in
// App.tsx so the gate always matches the page the handoff started from.
export const WELCOME_SETUP_PAGE_HEADINGS: Record<
  Exclude<WelcomeSetupPage, "welcome">,
  string
> = {
  "create-community": "Join a community",
  invite: "Redeem an invite",
  "nostr-key": "Use your existing key",
};

type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialTransitionMode?: WelcomeTransitionMode;
  onComplete: (community: Community, source: WelcomeSetupPage) => void;
};

const DEFAULT_COMMUNITY_HANDOFF_MIN_MS = 200;
const LOCAL_DEV_RELAY_URLS = new Set([
  "ws://localhost:3000",
  "ws://127.0.0.1:3000",
]);

function isLocalDevRelayUrl(relayUrl: string) {
  return LOCAL_DEV_RELAY_URLS.has(relayUrl.trim().replace(/\/$/, ""));
}

function wait(ms: number) {
  return new Promise<void>((resolve) => {
    window.setTimeout(resolve, ms);
  });
}

function NostrKeyImportPage({
  connectionError,
  disabled,
  onBack,
  onImport,
}: {
  connectionError: string | null;
  disabled: boolean;
  onBack: () => void;
  onImport: (nsec: string) => Promise<void>;
}) {
  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      direction="forward"
      transitionKey="nostr-key-forward"
    >
      <div className="w-full max-w-[440px]">
        <h1 className="text-3xl font-semibold tracking-tight">
          {WELCOME_SETUP_PAGE_HEADINGS["nostr-key"]}
        </h1>
        <p className="mt-3 text-sm leading-6 text-muted-foreground">
          Import your Nostr private key to use that identity with Buzz. If this
          key already has a profile on the relay, your name and avatar are
          restored automatically.
        </p>
      </div>

      <NostrKeyImportForm
        disabled={disabled}
        errorMessage={connectionError}
        onBack={onBack}
        onImport={onImport}
      />
    </OnboardingSlideTransition>
  );
}

export function WelcomeSetup({
  defaultRelayUrl,
  initialTransitionMode = "initial",
  onComplete,
}: WelcomeSetupProps) {
  const [page, setPage] = React.useState<WelcomeSetupPage>("welcome");
  const [transitionMode, setTransitionMode] =
    React.useState<WelcomeTransitionMode>(initialTransitionMode);
  const [isConnecting, setIsConnecting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [isRedeeming, setIsRedeeming] = React.useState(false);
  const [inviteError, setInviteError] = React.useState<string | null>(null);
  const systemColorScheme = useSystemColorScheme();

  const handleConnect = React.useCallback(
    async (relayUrl: string, communityName?: string, pubkey?: string) => {
      const trimmedUrl = relayUrl.trim();
      if (!trimmedUrl) {
        setError("Please enter a community URL.");
        return;
      }
      if (!communityName && isLocalDevRelayUrl(trimmedUrl)) {
        setError("Enter your relay URL to join a community.");
        setTransitionMode("forward");
        setPage("create-community");
        return;
      }

      const handoffStartedAt = performance.now();
      flushSync(() => {
        setIsConnecting(true);
        setError(null);
      });

      try {
        // We snapshot only the pubkey for display purposes (community switcher
        // labels, etc.). The private key lives on disk in `identity.key` and
        // is the single source of truth — never copied into localStorage.
        const identityPubkey = pubkey ?? (await getIdentity()).pubkey;
        const community = initFirstCommunity(
          trimmedUrl,
          identityPubkey,
          communityName,
        );

        if (!communityName) {
          const elapsedMs = performance.now() - handoffStartedAt;
          if (elapsedMs < DEFAULT_COMMUNITY_HANDOFF_MIN_MS) {
            await wait(DEFAULT_COMMUNITY_HANDOFF_MIN_MS - elapsedMs);
          }
        }

        // The parent moves this community into React state so first-run setup
        // can continue without a full page reload. The source page lets the
        // parent's loading gate keep matching the page the user came from.
        onComplete(community, page);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to connect. Try again.",
        );
        setIsConnecting(false);
      }
    },
    [onComplete, page],
  );

  const handleNostrImport = React.useCallback(
    async (nsec: string) => {
      const identity = await tauriImportIdentity(nsec);
      await handleConnect(defaultRelayUrl, undefined, identity.pubkey);
    },
    [defaultRelayUrl, handleConnect],
  );

  const handleWelcomeInviteRedeem = React.useCallback(
    async (relayWsUrl: string, code: string) => {
      setIsRedeeming(true);
      setInviteError(null);
      try {
        await claimInvite(relayWsUrl, code);
        await handleConnect(relayWsUrl);
      } catch (err) {
        setInviteError(inviteErrorMessage(err));
      } finally {
        setIsRedeeming(false);
      }
    },
    [handleConnect],
  );

  const showCreateCommunityPage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("create-community");
  }, []);

  const showInvitePage = React.useCallback(() => {
    setInviteError(null);
    setTransitionMode("forward");
    setPage("invite");
  }, []);

  const showNostrKeyPage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("nostr-key");
  }, []);

  const showWelcomePage = React.useCallback(() => {
    setError(null);
    setInviteError(null);
    setTransitionMode("backward");
    setPage("welcome");
  }, []);

  const currentStep =
    page === "welcome"
      ? isConnecting
        ? 2
        : 1
      : page === "nostr-key" || page === "invite"
        ? 1
        : 2;
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
        <StepProgress
          activeSegmentClassName="bg-primary"
          className="fixed bottom-12 left-1/2 z-40 -translate-x-1/2"
          completeSegmentClassName="bg-primary/35"
          currentStep={currentStep}
          inactiveSegmentClassName="bg-muted-foreground/25"
        />

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
              Choose your first community to get started.
            </p>

            <div className="mt-8 flex w-full flex-col gap-3">
              {isLocalDevRelayUrl(defaultRelayUrl) ? null : (
                <Button
                  className="h-10 w-full"
                  aria-disabled={isConnecting}
                  onClick={() => {
                    if (isConnecting) {
                      return;
                    }
                    setError(null);
                    void handleConnect(defaultRelayUrl);
                  }}
                  type="button"
                >
                  Continue with default community
                </Button>
              )}

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showCreateCommunityPage();
                }}
                type="button"
                variant="secondary"
              >
                Join a community
              </Button>

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showInvitePage();
                }}
                type="button"
                variant="ghost"
              >
                Have an invite?
              </Button>

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                data-testid="welcome-continue-nostr"
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showNostrKeyPage();
                }}
                type="button"
                variant="ghost"
              >
                I already have a key
              </Button>
            </div>

            {error ? (
              <div className="mt-4 w-full">
                <p className="text-sm text-destructive">{error}</p>
              </div>
            ) : null}
          </OnboardingSlideTransition>
        ) : page === "create-community" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`create-community-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                {WELCOME_SETUP_PAGE_HEADINGS["create-community"]}
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Communities are where teammates and agents collaborate across
                channels, DMs, and shared projects.
              </p>
            </div>

            <div className="mt-8 w-full">
              <CommunityEditForm
                cancelLabel="Back"
                initialName=""
                initialRelayUrl=""
                isSubmitting={isConnecting}
                onCancel={showWelcomePage}
                onSubmit={(name, url) => {
                  void handleConnect(url, name);
                }}
                submitLabel="Join a community"
              />
              {error ? (
                <p className="mt-2 text-center text-sm text-destructive">
                  {error}
                </p>
              ) : null}
            </div>
          </OnboardingSlideTransition>
        ) : page === "invite" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`invite-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                {WELCOME_SETUP_PAGE_HEADINGS.invite}
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Paste an invite link or code from a relay admin to join their
                community.
              </p>
            </div>

            <div className="mt-8 w-full">
              <InviteRedeemForm
                defaultRelayUrl={
                  isLocalDevRelayUrl(defaultRelayUrl)
                    ? undefined
                    : defaultRelayUrl
                }
                error={inviteError}
                isRedeeming={isRedeeming}
                onCancel={showWelcomePage}
                onRedeem={(relayWsUrl, code) => {
                  void handleWelcomeInviteRedeem(relayWsUrl, code);
                }}
              />
            </div>
          </OnboardingSlideTransition>
        ) : (
          <NostrKeyImportPage
            connectionError={error}
            disabled={isConnecting}
            onBack={showWelcomePage}
            onImport={handleNostrImport}
          />
        )}
      </div>
    </div>
  );
}
