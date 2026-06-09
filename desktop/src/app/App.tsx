import { getCurrentWindow } from "@tauri-apps/api/window";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useState,
} from "react";

import { router } from "@/app/router";
import { useReloadShortcut } from "@/app/useReloadShortcut";
import { useAppOnboardingState } from "@/features/onboarding/hooks";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import { useWorkspaceInit } from "@/features/workspaces/useWorkspaceInit";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { WelcomeSetup } from "@/features/workspaces/ui/WelcomeSetup";
import { createSproutQueryClient } from "@/shared/api/queryClient";
import { isSharedIdentity as isSharedIdentityCmd } from "@/shared/api/tauri";
import { listenForDeepLinks } from "@/shared/deep-link";

const LOADING_TEXT = "Setting up your workspace...";

function AppLoadingGate() {
  return (
    <div
      className="sprout-setup-loading-shell flex min-h-dvh flex-col items-center justify-center overflow-hidden px-6 py-10"
      data-testid="app-loading-gate"
      role="status"
    >
      <h1
        aria-live="polite"
        className="mt-6 text-center text-3xl font-semibold tracking-tight text-foreground"
      >
        <span className="sr-only">{LOADING_TEXT}</span>
        <span aria-hidden="true" className="sprout-setup-loading-text">
          {LOADING_TEXT}
        </span>
      </h1>
    </div>
  );
}

function WorkspaceQueryProvider({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createSproutQueryClient);

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function AppReady({ isSharedIdentity }: { isSharedIdentity: boolean }) {
  const onboarding = useAppOnboardingState(isSharedIdentity);

  if (onboarding.stage === "onboarding") {
    return (
      <OnboardingFlow
        actions={onboarding.flow.actions}
        initialProfile={onboarding.flow.initialProfile}
        key={onboarding.currentPubkey ?? "anonymous"}
      />
    );
  }

  if (onboarding.stage === "blocking") {
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
    switchWorkspace,
    reconnectWorkspace,
  } = useWorkspaces();

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
  // Composite key: changes when workspace ID changes OR when
  // the active workspace's config is updated (relayUrl/token).
  const workspaceKey = `${activeWorkspace?.id ?? "none"}-${reinitKey}`;
  const workspace = useWorkspaceInit(
    activeWorkspace,
    workspaceKey,
    sharedIdentity ?? false,
  );

  const handleSetupComplete = useCallback(() => {
    // Force a full reload so useWorkspaces re-initializes from localStorage.
    // This only runs once — during first-run setup when no workspace existed.
    window.location.reload();
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
        onComplete={handleSetupComplete}
      />
    );
  }

  // Wait for this exact workspace config to be applied to the backend before
  // rendering anything that connects to the relay. The appliedKey check avoids
  // a one-render race where React sees the new active workspace while the Tauri
  // backend is still configured for the previous one.
  if (!workspace.isReady || workspace.appliedKey !== workspaceKey) {
    return <AppLoadingGate />;
  }

  return (
    <WorkspaceQueryProvider key={workspaceKey}>
      <AppReady key={workspaceKey} isSharedIdentity={sharedIdentity} />
    </WorkspaceQueryProvider>
  );
}
