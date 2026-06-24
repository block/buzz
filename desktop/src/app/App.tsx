import { getCurrentWindow } from "@tauri-apps/api/window";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { Hexagon } from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useState,
} from "react";

import { router } from "@/app/router";
import { ThemeGrainientBackground } from "@/app/ThemeGrainientBackground";
import { useReloadShortcut } from "@/app/useReloadShortcut";
import { useAppOnboardingState } from "@/features/onboarding/hooks";
import { OnboardingSlideTransition } from "@/features/onboarding/ui/OnboardingSlideTransition";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import type { Workspace } from "@/features/workspaces/types";
import { useWorkspaceInit } from "@/features/workspaces/useWorkspaceInit";
import { useNestNotifications } from "@/features/workspaces/useNestNotifications";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { WelcomeSetup } from "@/features/workspaces/ui/WelcomeSetup";
import { createBuzzQueryClient } from "@/shared/api/queryClient";
import { isSharedIdentity as isSharedIdentityCmd } from "@/shared/api/tauri";
import { listenForDeepLinks } from "@/shared/deep-link";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";

const LOADING_TEXT = "Setting up your workspace...";

function AppLoadingGate() {
  return (
    <div
      className="buzz-setup-loading-shell flex min-h-dvh flex-col items-center justify-center overflow-hidden px-6 py-10"
      data-testid="app-loading-gate"
      role="status"
    >
      <StartupWindowDragRegion />
      <ThemeGrainientBackground />

      <h1
        aria-live="polite"
        className="relative z-10 mt-6 text-center text-3xl font-semibold text-foreground"
      >
        <span className="sr-only">{LOADING_TEXT}</span>
        <span aria-hidden="true" className="buzz-setup-loading-text">
          {LOADING_TEXT}
        </span>
      </h1>
    </div>
  );
}

function OnboardingLoadingGate() {
  const systemColorScheme = useSystemColorScheme();

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
          currentStep={2}
          inactiveSegmentClassName="bg-muted-foreground/25"
        />

        <OnboardingSlideTransition
          className="flex w-full flex-col items-center text-center"
          direction="forward"
          effect="none"
          transitionKey="workspace-connecting"
        >
          <div className="flex h-14 w-14 items-center justify-center rounded-lg border border-border bg-background text-foreground shadow-xs">
            <Hexagon className="h-7 w-7" aria-hidden="true" />
          </div>

          <h1 className="mt-6 text-3xl font-semibold tracking-tight">
            Welcome to Buzz
          </h1>
          <p className="mt-3 max-w-[440px] text-sm leading-6 text-muted-foreground">
            Choose your first workspace to get started.
          </p>

          <div className="mt-8 flex w-full max-w-[500px] flex-col gap-3">
            <Button
              aria-disabled="true"
              className="h-10 w-full"
              tabIndex={-1}
              type="button"
            >
              Continue with Block Inc. workspace
            </Button>

            <Button
              aria-disabled="true"
              className="h-10 w-full"
              tabIndex={-1}
              type="button"
              variant="secondary"
            >
              Join a workspace
            </Button>

            <Button
              aria-disabled="true"
              className="h-10 w-full"
              data-testid="welcome-continue-nostr"
              tabIndex={-1}
              type="button"
              variant="ghost"
            >
              I already have a key
            </Button>
          </div>
        </OnboardingSlideTransition>
      </div>
    </div>
  );
}

function WorkspaceQueryProvider({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createBuzzQueryClient);

  useEffect(() => {
    const e2eWindow = window as Window & {
      __BUZZ_E2E__?: unknown;
      __BUZZ_E2E_QUERY_CLIENT__?: typeof queryClient;
    };
    if (!e2eWindow.__BUZZ_E2E__) {
      return;
    }

    e2eWindow.__BUZZ_E2E_QUERY_CLIENT__ = queryClient;
    return () => {
      if (e2eWindow.__BUZZ_E2E_QUERY_CLIENT__ === queryClient) {
        delete e2eWindow.__BUZZ_E2E_QUERY_CLIENT__;
      }
    };
  }, [queryClient]);

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function AppReady({
  canBackToWorkspaceSetup,
  isCompletingFirstRunWorkspace,
  isSharedIdentity,
  onFirstRunWorkspaceSettled,
  onBackToWorkspaceSetup,
}: {
  canBackToWorkspaceSetup: boolean;
  isCompletingFirstRunWorkspace: boolean;
  isSharedIdentity: boolean;
  onFirstRunWorkspaceSettled: () => void;
  onBackToWorkspaceSetup: () => void;
}) {
  const onboarding = useAppOnboardingState(isSharedIdentity);

  useEffect(() => {
    if (isCompletingFirstRunWorkspace && onboarding.stage !== "blocking") {
      onFirstRunWorkspaceSettled();
    }
  }, [
    isCompletingFirstRunWorkspace,
    onboarding.stage,
    onFirstRunWorkspaceSettled,
  ]);

  if (onboarding.stage === "onboarding") {
    return (
      <OnboardingFlow
        actions={onboarding.flow.actions}
        canBackToWorkspaceSetup={canBackToWorkspaceSetup}
        initialProfile={onboarding.flow.initialProfile}
        key={onboarding.currentPubkey ?? "anonymous"}
        onBackToWorkspaceSetup={onBackToWorkspaceSetup}
      />
    );
  }

  if (onboarding.stage === "blocking") {
    if (isCompletingFirstRunWorkspace) {
      return <OnboardingLoadingGate />;
    }

    return <AppLoadingGate />;
  }

  return <RouterProvider router={router} />;
}

export function App() {
  // Mounted at the root so Cmd/Ctrl+R reloads in every app state,
  // including the loading and first-run setup screens below.
  useReloadShortcut();

  useLayoutEffect(() => {
    void getCurrentWindow().show();
  }, []);

  const [sharedIdentity, setSharedIdentity] = useState<boolean | null>(null);
  useEffect(() => {
    isSharedIdentityCmd()
      .then(setSharedIdentity)
      .catch((err) => {
        console.warn("is_shared_identity command failed:", err);
        setSharedIdentity(false);
      });
  }, []);

  const {
    activeWorkspace,
    reinitKey,
    addWorkspace,
    clearWorkspaces,
    switchWorkspace,
    reconnectWorkspace,
  } = useWorkspaces();
  const [isCompletingFirstRunWorkspace, setIsCompletingFirstRunWorkspace] =
    useState(false);
  const [canBackToWorkspaceSetup, setCanBackToWorkspaceSetup] = useState(false);
  const [welcomeTransitionMode, setWelcomeTransitionMode] = useState<
    "initial" | "backward"
  >("initial");

  useEffect(() => {
    const unlisten = listenForDeepLinks({
      addWorkspace,
      switchWorkspace,
      reconnectWorkspace,
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [addWorkspace, switchWorkspace, reconnectWorkspace]);
  // Surface nest-related backend events (repos-dir errors, legacy migration)
  // as toasts. Mounted before useWorkspaceInit so the listeners are registered
  // ahead of the first apply_workspace call.
  useNestNotifications();

  // Composite key: changes when workspace ID changes OR when
  // the active workspace's config is updated (relayUrl/token).
  const workspaceKey = `${activeWorkspace?.id ?? "none"}-${reinitKey}`;
  const workspace = useWorkspaceInit(
    activeWorkspace,
    workspaceKey,
    sharedIdentity ?? false,
  );

  const handleSetupComplete = useCallback(
    (workspace: Workspace) => {
      setWelcomeTransitionMode("initial");
      setIsCompletingFirstRunWorkspace(true);
      setCanBackToWorkspaceSetup(true);
      const workspaceId = addWorkspace(workspace);
      switchWorkspace(workspaceId);
    },
    [addWorkspace, switchWorkspace],
  );

  const handleBackToWorkspaceSetup = useCallback(() => {
    setWelcomeTransitionMode("backward");
    setIsCompletingFirstRunWorkspace(false);
    setCanBackToWorkspaceSetup(false);
    clearWorkspaces();
  }, [clearWorkspaces]);

  const handleFirstRunWorkspaceSettled = useCallback(() => {
    setIsCompletingFirstRunWorkspace(false);
  }, []);

  // Wait for the shared-identity IPC call to resolve before rendering
  // anything that depends on it. Without this gate, children briefly see
  // isSharedIdentity=false and may flash WelcomeSetup or the onboarding flow.
  if (sharedIdentity === null) {
    return <AppLoadingGate />;
  }

  // Show welcome setup for first-run users with no workspaces
  if (workspace.needsSetup) {
    return (
      <WelcomeSetup
        defaultRelayUrl={workspace.defaultRelayUrl}
        initialTransitionMode={welcomeTransitionMode}
        onComplete={handleSetupComplete}
      />
    );
  }

  // Wait for this exact workspace config to be applied to the backend before
  // rendering anything that connects to the relay. The appliedKey check avoids
  // a one-render race where React sees the new active workspace while the Tauri
  // backend is still configured for the previous one.
  if (!workspace.isReady || workspace.appliedKey !== workspaceKey) {
    if (isCompletingFirstRunWorkspace) {
      return <OnboardingLoadingGate />;
    }

    return <AppLoadingGate />;
  }

  return (
    <WorkspaceQueryProvider key={workspaceKey}>
      <AppReady
        canBackToWorkspaceSetup={canBackToWorkspaceSetup}
        isCompletingFirstRunWorkspace={isCompletingFirstRunWorkspace}
        key={workspaceKey}
        isSharedIdentity={sharedIdentity}
        onFirstRunWorkspaceSettled={handleFirstRunWorkspaceSettled}
        onBackToWorkspaceSetup={handleBackToWorkspaceSetup}
      />
    </WorkspaceQueryProvider>
  );
}
