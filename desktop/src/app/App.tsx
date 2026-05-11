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
import { useAppOnboardingState } from "@/features/onboarding/hooks";
import { OnboardingFlow } from "@/features/onboarding/ui/OnboardingFlow";
import { useWorkspaceInit } from "@/features/workspaces/useWorkspaceInit";
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { WelcomeSetup } from "@/features/workspaces/ui/WelcomeSetup";
import { createSproutQueryClient } from "@/shared/api/queryClient";
import { listenForDeepLinks } from "@/shared/deep-link";

function AppLoadingGate() {
  return (
    <div className="flex min-h-dvh items-center justify-center bg-[radial-gradient(circle_at_top,hsl(var(--primary)/0.14),transparent_48%),linear-gradient(180deg,hsl(var(--background)),hsl(var(--muted)/0.55))] px-4 py-8">
      <div className="w-full max-w-sm rounded-[28px] border border-border/70 bg-background/92 p-8 shadow-2xl backdrop-blur">
        <p className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">
          Sprout
        </p>
        <h1 className="mt-3 text-2xl font-semibold tracking-tight text-foreground">
          Checking your setup
        </h1>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          One sec while we load your profile.
        </p>
      </div>
    </div>
  );
}

function WorkspaceQueryProvider({ children }: { children: ReactNode }) {
  const [queryClient] = useState(createSproutQueryClient);

  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

function AppReady() {
  const onboarding = useAppOnboardingState();

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
  useLayoutEffect(() => {
    void getCurrentWindow().show();
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
  const workspace = useWorkspaceInit(activeWorkspace, workspaceKey);

  const handleSetupComplete = useCallback(() => {
    // Force a full reload so useWorkspaces re-initializes from localStorage.
    // This only runs once — during first-run setup when no workspace existed.
    window.location.reload();
  }, []);

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
      <AppReady key={workspaceKey} />
    </WorkspaceQueryProvider>
  );
}
