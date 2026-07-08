import { getCurrentWindow } from "@tauri-apps/api/window";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { Hexagon } from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import { router } from "@/app/router";
import { ThemeGrainientBackground } from "@/app/ThemeGrainientBackground";
import { useReloadShortcut } from "@/app/useReloadShortcut";
import { useAppOnboardingState } from "@/features/onboarding/hooks";
import { OnboardingSlideTransition } from "@/features/onboarding/ui/OnboardingSlideTransition";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import { KeyringLockedScreen } from "@/features/onboarding/ui/KeyringLockedScreen";
import { RelaunchRequiredScreen } from "@/features/onboarding/ui/RelaunchRequiredScreen";
import type { Workspace } from "@/features/workspaces/types";
import { useWorkspaceInit } from "@/features/workspaces/useWorkspaceInit";
import { useNestNotifications } from "@/features/workspaces/useNestNotifications";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { WelcomeSetup } from "@/features/workspaces/ui/WelcomeSetup";
import { createBuzzQueryClient } from "@/shared/api/queryClient";
import { isSharedIdentity as isSharedIdentityCmd } from "@/shared/api/tauri";
import { listenForDeepLinks } from "@/shared/deep-link";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";

const LOADING_TEXT = "Setting up your workspace...";

// Minimum time the cold-boot splash stays on screen. A real boot resolves the
// workspace in well under 100ms, and the hidden Tauri window (visible:false
// until getCurrentWindow().show()) takes longer than that to put its first
// frame on screen — without a hold, the bee is unmounted before it is ever
// visible. The hold runs as an overlay above the already-mounted app, so
// time-to-interactive is unchanged; only the reveal waits.
const BOOT_SPLASH_MIN_VISIBLE_MS = 1_200;
const BOOT_SPLASH_FADE_MS = 200;

type BootSplashPhase = "holding" | "fading" | "done";

// E2E runs skip the hold (it would slow every spec's boot and block pointer
// actionability); a spec can opt back in via __BUZZ_E2E__.bootSplashHoldMs.
function bootSplashHoldMs(): number {
  const e2e = (
    window as Window & {
      __BUZZ_E2E__?: { bootSplashHoldMs?: number };
    }
  ).__BUZZ_E2E__;
  if (e2e) {
    return e2e.bootSplashHoldMs ?? 0;
  }
  return BOOT_SPLASH_MIN_VISIBLE_MS;
}

function useBootSplashHold(): BootSplashPhase {
  const [phase, setPhase] = useState<BootSplashPhase>(() =>
    bootSplashHoldMs() > 0 ? "holding" : "done",
  );

  useEffect(() => {
    const holdMs = bootSplashHoldMs();
    if (holdMs <= 0) {
      return;
    }
    const fadeTimer = window.setTimeout(() => setPhase("fading"), holdMs);
    const doneTimer = window.setTimeout(
      () => setPhase("done"),
      holdMs + BOOT_SPLASH_FADE_MS,
    );
    return () => {
      window.clearTimeout(fadeTimer);
      window.clearTimeout(doneTimer);
    };
  }, []);

  return phase;
}

// Animated Buzz mark for the loading gates. The static BuzzMark renders in
// normal flow and sizes the box — it's plain SVG (no JS/SMIL), so it paints on
// the very first frame even before scripting starts, avoiding a blank flash on
// hard reload. The animated FuzzyLogo is layered on top and takes over once it
// begins playing.
function BeeLoader({
  ariaLabel,
  className,
  tintClassName = "text-foreground",
}: {
  ariaLabel: string;
  className?: string;
  tintClassName?: string;
}) {
  return (
    <div className={cn("relative", tintClassName, className)}>
      <BuzzMark className="block h-auto w-full" />
      <FuzzyLogo
        ariaLabel={ariaLabel}
        className="absolute inset-0 h-full! w-full! [&>svg]:h-full [&>svg]:w-full [&>svg]:max-w-full"
        fuzz
        loop
        loopRestSeconds={0}
      />
    </div>
  );
}

// Cold boot gate: the theme-adaptive grainient background with a single
// centered Buzz bee flying over it — the same static mark as before, now with
// its wings flapping (ported from the Buzz website's wing-flap). Replaces the
// old "Setting up your workspace" text, which stays as an sr-only caption.
function AppLoadingGate() {
  return (
    <div
      className="buzz-setup-loading-shell flex min-h-dvh flex-col items-center justify-center overflow-hidden px-6 py-10"
      data-testid="app-loading-gate"
      role="status"
    >
      <StartupWindowDragRegion />
      <ThemeGrainientBackground />
      <span className="sr-only">{LOADING_TEXT}</span>
      <FlappingBee className="relative z-10 h-auto w-28" />
    </div>
  );
}

// Quiet gate for switching between already-set-up workspaces: visually empty
// unless the switch takes long, so fast switches don't flash the boot splash.
function WorkspaceSwitchGate() {
  const [showSpinner, setShowSpinner] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => setShowSpinner(true), 300);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div
      className="flex min-h-dvh items-center justify-center bg-background"
      data-testid="workspace-switch-gate"
      role="status"
    >
      <StartupWindowDragRegion />
      <span className="sr-only">Switching workspace…</span>
      {showSpinner ? (
        <BeeLoader
          ariaLabel="Switching workspace…"
          className="h-auto w-20"
          tintClassName="text-muted-foreground"
        />
      ) : null}
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
              Continue with default workspace
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
  isWorkspaceSwitch,
  onFirstRunWorkspaceSettled,
  onBackToWorkspaceSetup,
}: {
  canBackToWorkspaceSetup: boolean;
  isCompletingFirstRunWorkspace: boolean;
  isSharedIdentity: boolean;
  isWorkspaceSwitch: boolean;
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

  if (onboarding.stage === "keyring-locked") {
    return <KeyringLockedScreen />;
  }

  if (onboarding.stage === "relaunch-required") {
    return <RelaunchRequiredScreen />;
  }

  if (onboarding.stage === "onboarding") {
    return (
      <OnboardingFlow
        actions={onboarding.flow.actions}
        canBackToWorkspaceSetup={canBackToWorkspaceSetup}
        identityLost={onboarding.identityLost}
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

    return isWorkspaceSwitch ? <WorkspaceSwitchGate /> : <AppLoadingGate />;
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

  // Latch once the workspace key deviates from its cold-boot value: from then
  // on, loading phases are in-app switches and get the quiet gate instead of
  // the full "Setting up your workspace" splash.
  const initialWorkspaceKeyRef = useRef(workspaceKey);
  const hasSwitchedWorkspaceRef = useRef(false);
  if (workspaceKey !== initialWorkspaceKeyRef.current) {
    hasSwitchedWorkspaceRef.current = true;
  }
  const isWorkspaceSwitch = hasSwitchedWorkspaceRef.current;

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

  const bootSplashPhase = useBootSplashHold();

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

    return isWorkspaceSwitch ? <WorkspaceSwitchGate /> : <AppLoadingGate />;
  }

  // The app mounts (and starts loading data) beneath the splash overlay; the
  // overlay just keeps the bee on screen long enough to be seen, then fades.
  // Workspace switches and first-run completion keep their quiet gates.
  const showBootSplashOverlay =
    bootSplashPhase !== "done" &&
    !isWorkspaceSwitch &&
    !isCompletingFirstRunWorkspace;

  return (
    <WorkspaceQueryProvider key={workspaceKey}>
      <AppReady
        canBackToWorkspaceSetup={canBackToWorkspaceSetup}
        isCompletingFirstRunWorkspace={isCompletingFirstRunWorkspace}
        isWorkspaceSwitch={isWorkspaceSwitch}
        key={workspaceKey}
        isSharedIdentity={sharedIdentity}
        onFirstRunWorkspaceSettled={handleFirstRunWorkspaceSettled}
        onBackToWorkspaceSetup={handleBackToWorkspaceSetup}
      />
      {showBootSplashOverlay ? (
        <div
          aria-hidden="true"
          className={cn(
            "fixed inset-0 z-50 transition-opacity",
            bootSplashPhase === "fading" ? "opacity-0" : "opacity-100",
          )}
          data-testid="boot-splash-overlay"
          style={{ transitionDuration: `${BOOT_SPLASH_FADE_MS}ms` }}
        >
          <AppLoadingGate />
        </div>
      ) : null}
    </WorkspaceQueryProvider>
  );
}
