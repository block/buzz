import { isTauri } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
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
import { KnownAgentPubkeysProvider } from "@/features/agents/useKnownAgentPubkeys";
import { useAppOnboardingState } from "@/features/onboarding/hooks";
import { OnboardingSlideTransition } from "@/features/onboarding/ui/OnboardingSlideTransition";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import { KeyringLockedScreen } from "@/features/onboarding/ui/KeyringLockedScreen";
import { RelaunchRequiredScreen } from "@/features/onboarding/ui/RelaunchRequiredScreen";
import { ResetFailedScreen } from "@/features/onboarding/ui/ResetFailedScreen";
import type { Community } from "@/features/communities/types";
import { useCommunityInit } from "@/features/communities/useCommunityInit";
import { useNestNotifications } from "@/features/communities/useNestNotifications";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  WELCOME_SETUP_PAGE_HEADINGS,
  WelcomeSetup,
  type WelcomeSetupPage,
} from "@/features/communities/ui/WelcomeSetup";
import { CommunityApplyErrorScreen } from "@/features/communities/ui/CommunityApplyErrorScreen";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";
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

const LOADING_TEXT = "Setting up your community...";

// Minimum time the cold-boot splash stays on screen. A real boot resolves the
// community in well under 100ms, and the native window setup plus first paint
// can take longer than that — without a hold, the bee is unmounted before it is
// ever visible. The hold runs as an overlay above the already-mounted app, so
// time-to-interactive is unchanged; only the reveal waits.
const BOOT_SPLASH_MIN_VISIBLE_MS = 1_200;
const BOOT_SPLASH_FADE_MS = 200;
const INITIAL_RENDER_READY_EVENT = "initial-render-ready";

type BootSplashPhase = "holding" | "fading" | "done";

function useInitialRenderReady() {
  useLayoutEffect(() => {
    if (!isTauri()) {
      return;
    }

    void emit(INITIAL_RENDER_READY_EVENT);
  }, []);
}

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
// old "Setting up your community" text, which stays as an sr-only caption.
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

// Quiet gate for switching between already-set-up communities: visually empty
// unless the switch takes long, so fast switches don't flash the boot splash.
function CommunitySwitchGate() {
  const [showSpinner, setShowSpinner] = useState(false);

  useEffect(() => {
    const timer = window.setTimeout(() => setShowSpinner(true), 300);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div
      className="flex min-h-dvh items-center justify-center bg-background"
      data-testid="community-switch-gate"
      role="status"
    >
      <StartupWindowDragRegion />
      <span className="sr-only">Switching community…</span>
      {showSpinner ? (
        <BeeLoader
          ariaLabel="Switching community…"
          className="h-auto w-20"
          tintClassName="text-muted-foreground"
        />
      ) : null}
    </div>
  );
}

// Shown while a first-run community handoff settles (config apply + relay
// round trips). For the default-community path the user pressed a button on
// the welcome page itself, so the gate keeps rendering that same page with
// the buttons disabled — seamless continuity. Every other WelcomeSetup
// sub-page (key import, invite, join community) instead gets a forward-motion
// connecting view under its own heading; showing the welcome replica there
// reads as being kicked back to step 1.
function OnboardingLoadingGate({ source }: { source: WelcomeSetupPage }) {
  const systemColorScheme = useSystemColorScheme();
  const connectingHeading =
    source !== "welcome" ? WELCOME_SETUP_PAGE_HEADINGS[source] : null;

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

        {connectingHeading !== null ? (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction="forward"
            effect="none"
            transitionKey="community-connecting"
          >
            <div
              className="flex w-full max-w-[440px] flex-col items-center"
              data-testid="onboarding-connecting-gate"
              role="status"
            >
              <h1 className="text-3xl font-semibold tracking-tight">
                {connectingHeading}
              </h1>
              <BeeLoader
                ariaLabel="Connecting to your community…"
                className="mt-12 h-auto w-20"
                tintClassName="text-muted-foreground"
              />
              <p className="mt-6 text-sm leading-6 text-muted-foreground">
                Connecting to your community…
              </p>
            </div>
          </OnboardingSlideTransition>
        ) : (
          <OnboardingSlideTransition
            className="flex w-full flex-col items-center text-center"
            direction="forward"
            effect="none"
            transitionKey="community-connecting"
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

            <div className="mt-8 flex w-full max-w-[500px] flex-col gap-3">
              <Button
                aria-disabled="true"
                className="h-10 w-full"
                tabIndex={-1}
                type="button"
              >
                Continue with default community
              </Button>

              <Button
                aria-disabled="true"
                className="h-10 w-full"
                tabIndex={-1}
                type="button"
                variant="secondary"
              >
                Join a community
              </Button>

              <Button
                aria-disabled="true"
                className="h-10 w-full"
                tabIndex={-1}
                type="button"
                variant="ghost"
              >
                Have an invite?
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
        )}
      </div>
    </div>
  );
}

function CommunityQueryProvider({ children }: { children: ReactNode }) {
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
  firstRunHandoffSource,
  isSharedIdentity,
  isCommunitySwitch,
  onFirstRunCommunitySettled,
}: {
  firstRunHandoffSource: WelcomeSetupPage | null;
  isSharedIdentity: boolean;
  isCommunitySwitch: boolean;
  onFirstRunCommunitySettled: () => void;
}) {
  const onboarding = useAppOnboardingState(isSharedIdentity);

  useEffect(() => {
    if (firstRunHandoffSource !== null && onboarding.stage !== "blocking") {
      onFirstRunCommunitySettled();
    }
  }, [firstRunHandoffSource, onboarding.stage, onFirstRunCommunitySettled]);

  if (onboarding.stage === "reset-failed") {
    return <ResetFailedScreen />;
  }

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
        identityLost={onboarding.identityLost}
        initialProfile={onboarding.flow.initialProfile}
        key={onboarding.currentPubkey ?? "anonymous"}
      />
    );
  }

  if (onboarding.stage === "blocking") {
    if (firstRunHandoffSource !== null) {
      return <OnboardingLoadingGate source={firstRunHandoffSource} />;
    }

    return isCommunitySwitch ? <CommunitySwitchGate /> : <AppLoadingGate />;
  }

  return (
    <KnownAgentPubkeysProvider>
      <RouterProvider router={router} />
    </KnownAgentPubkeysProvider>
  );
}

export function App() {
  // Mounted at the root so Cmd/Ctrl+R reloads in every app state,
  // including the loading and first-run setup screens below.
  useReloadShortcut();
  useInitialRenderReady();

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
    activeCommunity,
    reinitKey,
    addCommunity,
    switchCommunity,
    reconnectCommunity,
  } = useCommunities();
  // Non-null while a first-run community handoff settles; the value is the
  // WelcomeSetup sub-page it started from, which picks the loading gate.
  const [firstRunHandoffSource, setFirstRunHandoffSource] =
    useState<WelcomeSetupPage | null>(null);
  const [isCommunityChangeOpen, setIsCommunityChangeOpen] = useState(false);

  useEffect(() => {
    const unlisten = listenForDeepLinks({
      addCommunity,
      switchCommunity,
      reconnectCommunity,
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [addCommunity, switchCommunity, reconnectCommunity]);
  // Surface nest-related backend events (repos-dir errors, legacy migration)
  // as toasts. Mounted before useCommunityInit so the listeners are registered
  // ahead of the first apply_workspace call.
  useNestNotifications();

  // Composite key: changes when community ID changes OR when
  // the active community's config is updated (relayUrl/token).
  const communityKey = `${activeCommunity?.id ?? "none"}-${reinitKey}`;

  // Latch once the community key deviates from its cold-boot value: from then
  // on, loading phases are in-app switches and get the quiet gate instead of
  // the full "Setting up your community" splash.
  const initialCommunityKeyRef = useRef(communityKey);
  const hasSwitchedCommunityRef = useRef(false);
  if (communityKey !== initialCommunityKeyRef.current) {
    hasSwitchedCommunityRef.current = true;
  }
  const isCommunitySwitch = hasSwitchedCommunityRef.current;

  const community = useCommunityInit(
    activeCommunity,
    communityKey,
    sharedIdentity ?? false,
  );

  const handleSetupComplete = useCallback(
    (community: Community, source: WelcomeSetupPage) => {
      setFirstRunHandoffSource(source);
      const communityId = addCommunity(community);
      switchCommunity(communityId);
    },
    [addCommunity, switchCommunity],
  );

  const handleFirstRunCommunitySettled = useCallback(() => {
    setFirstRunHandoffSource(null);
  }, []);

  const bootSplashPhase = useBootSplashHold();

  // Wait for the shared-identity IPC call to resolve before rendering
  // anything that depends on it. Without this gate, children briefly see
  // isSharedIdentity=false and may flash WelcomeSetup or the onboarding flow.
  if (sharedIdentity === null) {
    return <AppLoadingGate />;
  }

  // Show welcome setup for first-run users with no communities
  if (community.needsSetup) {
    return (
      <WelcomeSetup
        defaultRelayUrl={community.defaultRelayUrl}
        onComplete={handleSetupComplete}
      />
    );
  }

  // Surface apply failures so the user can retry or change community.
  if ("error" in community && community.error) {
    return (
      <>
        <CommunityApplyErrorScreen
          error={community.error}
          onChangeCommunity={() => setIsCommunityChangeOpen(true)}
          onRetry={reconnectCommunity}
        />
        {isCommunityChangeOpen ? (
          <CommunityChangeOverlay
            onClose={() => setIsCommunityChangeOpen(false)}
          />
        ) : null}
      </>
    );
  }

  // Wait for this exact community config to be applied to the backend before
  // rendering anything that connects to the relay. The appliedKey check avoids
  // a one-render race where React sees the new active community while the Tauri
  // backend is still configured for the previous one.
  if (!community.isReady || community.appliedKey !== communityKey) {
    if (firstRunHandoffSource !== null) {
      return <OnboardingLoadingGate source={firstRunHandoffSource} />;
    }

    return isCommunitySwitch ? <CommunitySwitchGate /> : <AppLoadingGate />;
  }

  // The app mounts (and starts loading data) beneath the splash overlay; the
  // overlay just keeps the bee on screen long enough to be seen, then fades.
  // Community switches and first-run completion keep their quiet gates.
  const showBootSplashOverlay =
    bootSplashPhase !== "done" &&
    !isCommunitySwitch &&
    firstRunHandoffSource === null;

  return (
    <CommunityQueryProvider key={communityKey}>
      <AppReady
        firstRunHandoffSource={firstRunHandoffSource}
        isCommunitySwitch={isCommunitySwitch}
        key={communityKey}
        isSharedIdentity={sharedIdentity}
        onFirstRunCommunitySettled={handleFirstRunCommunitySettled}
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
    </CommunityQueryProvider>
  );
}
