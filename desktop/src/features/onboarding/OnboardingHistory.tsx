import * as React from "react";

import { router } from "@/app/router";
import {
  createOnboardingHistoryState,
  onboardingRoutePath,
  readOnboardingHistoryEntry,
  type OnboardingHistoryEntry,
  type OnboardingRouteStep,
} from "./onboardingRoute";

type HistoryAction = "initial" | "PUSH" | "REPLACE" | "BACK" | "FORWARD" | "GO";

type OnboardingHistorySnapshot = {
  action: HistoryAction;
  depth: number;
  direction: "backward" | "forward";
  step: OnboardingRouteStep | null;
};

type OnboardingHistoryContextValue = OnboardingHistorySnapshot & {
  back: (fallback: OnboardingRouteStep) => void;
  backBy: (steps: number, fallback: OnboardingRouteStep) => void;
  exit: (path: string) => void;
  push: (step: OnboardingRouteStep) => void;
  reset: (step: OnboardingRouteStep) => void;
  replace: (step: OnboardingRouteStep) => void;
};

const OnboardingHistoryContext =
  React.createContext<OnboardingHistoryContextValue | null>(null);

function currentEntry(sessionId: string): OnboardingHistoryEntry | null {
  return readOnboardingHistoryEntry(
    router.history.location.pathname,
    router.history.location.state,
    sessionId,
  );
}

function readSnapshot(
  sessionId: string,
  action: HistoryAction,
  goIndex?: number,
): OnboardingHistorySnapshot {
  const entry = currentEntry(sessionId);
  const direction =
    action === "BACK" || (action === "GO" && (goIndex ?? 0) < 0)
      ? "backward"
      : "forward";
  return {
    action,
    depth: entry?.depth ?? 0,
    direction,
    step: entry?.step ?? null,
  };
}

export function OnboardingHistoryProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const sessionIdRef = React.useRef(crypto.randomUUID());
  const [snapshot, setSnapshot] = React.useState(() =>
    readSnapshot(sessionIdRef.current, "initial"),
  );

  React.useEffect(
    () =>
      router.history.subscribe(({ action }) => {
        setSnapshot(
          readSnapshot(
            sessionIdRef.current,
            action.type,
            action.type === "GO" ? action.index : undefined,
          ),
        );
      }),
    [],
  );

  const replace = React.useCallback((step: OnboardingRouteStep) => {
    const sessionId = sessionIdRef.current;
    const entry = currentEntry(sessionId);
    router.history.replace(
      onboardingRoutePath(step),
      createOnboardingHistoryState(sessionId, entry?.depth ?? 0),
    );
  }, []);

  const push = React.useCallback((step: OnboardingRouteStep) => {
    const sessionId = sessionIdRef.current;
    const entry = currentEntry(sessionId);
    router.history.push(
      onboardingRoutePath(step),
      createOnboardingHistoryState(sessionId, (entry?.depth ?? 0) + 1),
    );
  }, []);

  const reset = React.useCallback((step: OnboardingRouteStep) => {
    const sessionId = crypto.randomUUID();
    sessionIdRef.current = sessionId;
    const path = onboardingRoutePath(step);
    const state = createOnboardingHistoryState(sessionId, 0);
    // Replace the route being retired, then leave an equivalent current entry.
    // Browser Back lands on the fresh session boundary instead of reviving the
    // app or completed onboarding route that preceded it.
    router.history.replace(path, state);
    router.history.push(path, state);
  }, []);

  const backBy = React.useCallback(
    (steps: number, fallback: OnboardingRouteStep) => {
      if (snapshot.step && steps > 0 && snapshot.depth >= steps) {
        router.history.go(-steps);
        return;
      }
      replace(fallback);
    },
    [replace, snapshot.depth, snapshot.step],
  );

  const back = React.useCallback(
    (fallback: OnboardingRouteStep) => backBy(1, fallback),
    [backBy],
  );

  const exit = React.useCallback((path: string) => {
    // Retire every entry from the completed flow before replacing the current
    // route. A later browser Back can still reach an old URL, but its marker no
    // longer belongs to the active session and cannot reopen onboarding.
    sessionIdRef.current = crypto.randomUUID();
    router.history.replace(path, {});
  }, []);

  const value = React.useMemo<OnboardingHistoryContextValue>(
    () => ({
      ...snapshot,
      back,
      backBy,
      exit,
      push,
      reset,
      replace,
    }),
    [back, backBy, exit, push, reset, replace, snapshot],
  );

  return (
    <OnboardingHistoryContext.Provider value={value}>
      {children}
    </OnboardingHistoryContext.Provider>
  );
}

export function useOnboardingHistory() {
  const value = React.useContext(OnboardingHistoryContext);
  if (!value) {
    throw new Error(
      "useOnboardingHistory must be used within OnboardingHistoryProvider",
    );
  }
  return value;
}
