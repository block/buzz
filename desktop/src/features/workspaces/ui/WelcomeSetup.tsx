import * as React from "react";
import { flushSync } from "react-dom";

import {
  getIdentity,
  importIdentity as tauriImportIdentity,
} from "@/shared/api/tauriIdentity";
import { NostrKeyImportForm } from "@/features/onboarding/ui/NostrKeyImportForm";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "@/features/onboarding/ui/OnboardingSlideTransition";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";

import type { Workspace } from "../types";
import { initFirstWorkspace } from "../workspaceStorage";

type WelcomeSetupPage = "welcome" | "create-workspace" | "nostr-key";
type WelcomeTransitionMode = "initial" | OnboardingTransitionDirection;

type WelcomeSetupProps = {
  defaultRelayUrl: string;
  initialTransitionMode?: WelcomeTransitionMode;
  onComplete: (workspace: Workspace) => void;
};

const DEFAULT_WORKSPACE_HANDOFF_MIN_MS = 200;
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
          Use your existing key
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
  const [customWorkspaceName, setCustomWorkspaceName] = React.useState("");
  const [customRelayUrl, setCustomRelayUrl] = React.useState("");
  const [isConnecting, setIsConnecting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const systemColorScheme = useSystemColorScheme();

  const handleConnect = React.useCallback(
    async (relayUrl: string, workspaceName?: string, pubkey?: string) => {
      const trimmedUrl = relayUrl.trim();
      if (!trimmedUrl) {
        setError("Please enter a workspace URL.");
        return;
      }
      if (!workspaceName && isLocalDevRelayUrl(trimmedUrl)) {
        setError("Enter your relay URL to join a workspace.");
        setTransitionMode("forward");
        setPage("create-workspace");
        return;
      }

      const handoffStartedAt = performance.now();
      flushSync(() => {
        setIsConnecting(true);
        setError(null);
      });

      try {
        // We snapshot only the pubkey for display purposes (workspace switcher
        // labels, etc.). The private key lives on disk in `identity.key` and
        // is the single source of truth — never copied into localStorage.
        const identityPubkey = pubkey ?? (await getIdentity()).pubkey;
        const workspace = initFirstWorkspace(
          trimmedUrl,
          identityPubkey,
          workspaceName,
        );

        if (!workspaceName) {
          const elapsedMs = performance.now() - handoffStartedAt;
          if (elapsedMs < DEFAULT_WORKSPACE_HANDOFF_MIN_MS) {
            await wait(DEFAULT_WORKSPACE_HANDOFF_MIN_MS - elapsedMs);
          }
        }

        // The parent moves this workspace into React state so first-run setup
        // can continue without a full page reload.
        onComplete(workspace);
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to connect. Try again.",
        );
        setIsConnecting(false);
      }
    },
    [onComplete],
  );

  const handleNostrImport = React.useCallback(
    async (nsec: string) => {
      const identity = await tauriImportIdentity(nsec);
      await handleConnect(defaultRelayUrl, undefined, identity.pubkey);
    },
    [defaultRelayUrl, handleConnect],
  );

  const handleCustomWorkspaceSubmit = React.useCallback(
    (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const trimmedName = customWorkspaceName.trim();
      const trimmedUrl = customRelayUrl.trim();
      if (!trimmedName) {
        setError("Please enter a workspace name.");
        return;
      }
      if (!trimmedUrl) {
        setError("Please enter a workspace URL.");
        return;
      }
      void handleConnect(trimmedUrl, trimmedName);
    },
    [customRelayUrl, customWorkspaceName, handleConnect],
  );

  const showCreateWorkspacePage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("create-workspace");
  }, []);

  const showNostrKeyPage = React.useCallback(() => {
    setError(null);
    setTransitionMode("forward");
    setPage("nostr-key");
  }, []);

  const showWelcomePage = React.useCallback(() => {
    setError(null);
    setTransitionMode("backward");
    setPage("welcome");
  }, []);

  const currentStep =
    page === "welcome" ? (isConnecting ? 2 : 1) : page === "nostr-key" ? 1 : 2;
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
              Choose your first workspace to get started.
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
                  Continue with default workspace
                </Button>
              )}

              <Button
                className="h-10 w-full"
                aria-disabled={isConnecting}
                onClick={() => {
                  if (isConnecting) {
                    return;
                  }
                  showCreateWorkspacePage();
                }}
                type="button"
                variant="secondary"
              >
                Join a workspace
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
        ) : page === "create-workspace" ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction={transitionDirection}
            transitionKey={`create-workspace-${transitionDirection}`}
          >
            <div className="w-full max-w-[440px]">
              <h1 className="text-3xl font-semibold tracking-tight">
                Join a workspace
              </h1>
              <p className="mt-3 text-sm leading-6 text-muted-foreground">
                Workspaces are where teammates and agents collaborate across
                channels, DMs, and shared projects.
              </p>
            </div>

            <form
              className="mt-8 flex w-full flex-col gap-4"
              onSubmit={handleCustomWorkspaceSubmit}
            >
              <div className="space-y-1.5 text-left">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="workspace-name"
                >
                  Workspace name
                </label>
                <Input
                  autoFocus
                  className="h-10 bg-background"
                  id="workspace-name"
                  onChange={(event) => {
                    setCustomWorkspaceName(event.target.value);
                    setError(null);
                  }}
                  placeholder="Design team"
                  type="text"
                  value={customWorkspaceName}
                />
              </div>

              <div className="space-y-1.5 text-left">
                <label
                  className="text-sm font-medium text-foreground"
                  htmlFor="workspace-url"
                >
                  Workspace URL
                </label>
                <Input
                  className="h-10 bg-background"
                  id="workspace-url"
                  onChange={(event) => {
                    setCustomRelayUrl(event.target.value);
                    setError(null);
                  }}
                  placeholder="wss://relay.example.com"
                  type="text"
                  value={customRelayUrl}
                />
              </div>

              <div className="flex w-full flex-col gap-3 pt-1">
                <Button
                  className="h-10 w-full"
                  disabled={
                    isConnecting ||
                    !customWorkspaceName.trim() ||
                    !customRelayUrl.trim()
                  }
                  type="submit"
                >
                  {isConnecting ? (
                    <Spinner
                      aria-label="Joining workspace"
                      className="h-4 w-4 border-2"
                    />
                  ) : (
                    "Join a workspace"
                  )}
                </Button>

                <Button
                  className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                  disabled={isConnecting}
                  onClick={showWelcomePage}
                  type="button"
                  variant="ghost"
                >
                  Back
                </Button>

                {error ? (
                  <p className="text-center text-sm text-destructive">
                    {error}
                  </p>
                ) : null}
              </div>
            </form>
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
