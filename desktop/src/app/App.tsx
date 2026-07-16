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
import { useMachineOnboardingState } from "@/features/onboarding/machineOnboarding";
import { useCommunityOnboarding } from "@/features/onboarding/communityOnboarding";
import { CommunityOnboardingFlow } from "@/features/onboarding/ui/CommunityOnboardingFlow";
import { MachineOnboardingFlow } from "@/features/onboarding/ui/MachineOnboardingFlow";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import { PendingInviteGate } from "@/features/onboarding/ui/PendingInviteGate";
import { KeyringLockedScreen } from "@/features/onboarding/ui/KeyringLockedScreen";
import { RelaunchRequiredScreen } from "@/features/onboarding/ui/RelaunchRequiredScreen";
import { ResetFailedScreen } from "@/features/onboarding/ui/ResetFailedScreen";
import { useCommunityInit } from "@/features/communities/useCommunityInit";
import { useNestNotifications } from "@/features/communities/useNestNotifications";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  onAddCommunityPrefillAvailable,
  requestAddCommunityPrefill,
} from "@/features/communities/addCommunityPrefill";
import { WelcomeSetup } from "@/features/communities/ui/WelcomeSetup";
import { CommunityApplyErrorScreen } from "@/features/communities/ui/CommunityApplyErrorScreen";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";
import { createBuzzQueryClient } from "@/shared/api/queryClient";
import { isSharedIdentity as isSharedIdentityCmd } from "@/shared/api/tauri";
import { listenForDeepLinks } from "@/shared/deep-link";
import { cn } from "@/shared/lib/cn";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import { FlappingBee } from "@/shared/ui/buzz-logo/FlappingBee";
import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

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
  isSharedIdentity,
  isCommunitySwitch,
}: {
  isSharedIdentity: boolean;
  isCommunitySwitch: boolean;
}) {
  const onboarding = useAppOnboardingState(isSharedIdentity);

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
    return isCommunitySwitch ? <CommunitySwitchGate /> : <AppLoadingGate />;
  }

  return (
    <KnownAgentPubkeysProvider>
      <RouterProvider router={router} />
    </KnownAgentPubkeysProvider>
  );
}

function CommunityApp({ sharedIdentity }: { sharedIdentity: boolean }) {
  const {
    activeCommunity,
    reinitKey,
    addCommunity,
    switchCommunity,
    reconnectCommunity,
  } = useCommunities();
  const communityOnboarding = useCommunityOnboarding();
  const [isCommunityChangeOpen, setIsCommunityChangeOpen] = useState(false);

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
    sharedIdentity,
  );

  const handleCommunityOnboardingConnect = useCallback(() => {
    const transaction = communityOnboarding.transaction;
    if (transaction?.stage !== "connecting") return;
    if (transaction.communityId) {
      switchCommunity(transaction.communityId);
      return;
    }
    const id = addCommunity({
      id: crypto.randomUUID(),
      name: transaction.communityName,
      relayUrl: transaction.relayUrl,
      token: transaction.token,
      reposDir: transaction.reposDir,
      addedAt: new Date().toISOString(),
    });
    communityOnboarding.update({ communityId: id, error: undefined });
    switchCommunity(id);
    reconnectCommunity();
  }, [addCommunity, communityOnboarding, reconnectCommunity, switchCommunity]);

  const bootSplashPhase = useBootSplashHold();

  const transaction = communityOnboarding.transaction;
  const targetIsReady =
    transaction?.communityId === activeCommunity?.id &&
    community.isReady &&
    community.appliedKey === communityKey;
  useEffect(() => {
    if (transaction?.stage === "connecting" && targetIsReady) {
      communityOnboarding.update({ stage: "profile", error: undefined });
    }
  }, [communityOnboarding.update, targetIsReady, transaction?.stage]);
  if (transaction) {
    return (
      <CommunityOnboardingFlow onConnect={handleCommunityOnboardingConnect} />
    );
  }

  // Show welcome setup for first-run users with no communities
  if (community.needsSetup) {
    return <WelcomeSetup defaultRelayUrl={community.defaultRelayUrl} />;
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
    return isCommunitySwitch ? <CommunitySwitchGate /> : <AppLoadingGate />;
  }

  // The app mounts (and starts loading data) beneath the splash overlay; the
  // overlay just keeps the bee on screen long enough to be seen, then fades.
  // Community switches keep their quiet gate.
  const showBootSplashOverlay =
    bootSplashPhase !== "done" && !isCommunitySwitch;

  return (
    <CommunityQueryProvider key={communityKey}>
      <AppReady
        isCommunitySwitch={isCommunitySwitch}
        key={communityKey}
        isSharedIdentity={sharedIdentity}
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

function MachineBootstrap({ sharedIdentity }: { sharedIdentity: boolean }) {
  const { activeCommunity } = useCommunities();
  const communityOnboarding = useCommunityOnboarding();
  const machine = useMachineOnboardingState({
    hasConfiguredCommunity: activeCommunity !== null,
    isSharedIdentity: sharedIdentity,
  });

  // Deep links are captured here — above the machine-onboarding gate — not in
  // CommunityApp. The Rust side queues them; draining into the persisted
  // community-onboarding transaction immediately means an invite opened on a
  // fresh install is acknowledged on screen while the identity steps are
  // still pending, and survives a relaunch in between.
  useEffect(() => {
    const unlisten = listenForDeepLinks({
      startCommunityOnboarding: communityOnboarding.start,
      openAddCommunity: requestAddCommunityPrefill,
      onAddCommunityAvailable: onAddCommunityPrefillAvailable,
    });
    return () => {
      void unlisten.then((fn) => fn());
    };
  }, [communityOnboarding.start]);

  if (machine.stage === "reset-failed") return <ResetFailedScreen />;
  if (machine.stage === "keyring-locked") return <KeyringLockedScreen />;
  if (machine.stage === "relaunch-required") return <RelaunchRequiredScreen />;
  if (machine.stage === "blocking") return <AppLoadingGate />;
  if (machine.stage === "ready") {
    return <CommunityApp sharedIdentity={sharedIdentity} />;
  }

  // A community deep link that arrived before machine onboarding finished is
  // persisted immediately and acknowledged here. Invite claiming waits until
  // setup completes so it is signed only by the user's final identity.
  const transaction = communityOnboarding.transaction;
  const isDeepLink =
    transaction?.source === "deep-link-join" ||
    transaction?.source === "deep-link-connect";
  const shouldAcknowledgeDeepLink = isDeepLink && !transaction.acknowledged;

  return (
    <>
      <MachineOnboardingFlow
        complete={machine.complete}
        identityLost={machine.identityLost}
        queryClient={machine.queryClient}
      />
      {shouldAcknowledgeDeepLink ? <PendingInviteGate /> : null}
    </>
  );
}

export function App() {
  useReloadShortcut();
  useInitialRenderReady();
  const [sharedIdentity, setSharedIdentity] = useState<boolean | null>(null);
  const [queryClient] = useState(createBuzzQueryClient);

  useEffect(() => {
    isSharedIdentityCmd()
      .then(setSharedIdentity)
      .catch((err) => {
        console.warn("is_shared_identity command failed:", err);
        setSharedIdentity(false);
      });
  }, []);

  if (sharedIdentity === null) return <AppLoadingGate />;

  return (
    <QueryClientProvider client={queryClient}>
      <MachineBootstrap sharedIdentity={sharedIdentity} />
    </QueryClientProvider>
  );
}
