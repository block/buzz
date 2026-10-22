import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

export type SquareEmployeeSessionStatus = {
  required: boolean;
  authenticated: boolean;
  canRetry?: boolean;
  expiresAt: number | null;
};

function admitsCommunity(status: SquareEmployeeSessionStatus | null) {
  return (
    status !== null &&
    (!status.required ||
      (status.authenticated &&
        status.expiresAt !== null &&
        status.expiresAt * 1000 > Date.now()))
  );
}

/** Native session changes invalidate every webview before checking current access. */
export function useSquareEmployeeSession(active: boolean, scope: string) {
  const generation = useRef(0);
  const listening = useRef(false);
  const [listenerRevision, setListenerRevision] = useState(0);
  const retry = useCallback(
    () => setListenerRevision((value) => value + 1),
    [],
  );
  const [snapshot, setSnapshot] = useState<{
    scope: string;
    revision: number;
    status: SquareEmployeeSessionStatus | null;
    error: boolean;
  } | null>(null);
  const refresh = useCallback(() => {
    const ownGeneration = ++generation.current;
    setSnapshot({ scope, revision: ownGeneration, status: null, error: false });
    void invoke<SquareEmployeeSessionStatus>("employee_session_status")
      .then((status) => {
        if (generation.current === ownGeneration)
          setSnapshot({ scope, revision: ownGeneration, status, error: false });
      })
      .catch(() => {
        if (generation.current === ownGeneration)
          setSnapshot({
            scope,
            revision: ownGeneration,
            status: null,
            error: true,
          });
      });
  }, [scope]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: explicit retry must re-register a failed native event listener.
  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    window.addEventListener("square-employee-auth-changed", refresh);
    void listen("builderlab-session-changed", () => {
      if (!disposed) refresh();
    })
      .then((stop) => {
        if (disposed) {
          stop();
          return;
        }
        unlisten = stop;
        listening.current = true;
        // Register first, then read: a mutation during registration cannot be lost.
        refresh();
      })
      .catch(() => {
        if (!disposed) {
          const revision = ++generation.current;
          setSnapshot({ scope, revision, status: null, error: true });
        }
      });
    return () => {
      disposed = true;
      listening.current = false;
      generation.current += 1;
      window.removeEventListener("square-employee-auth-changed", refresh);
      unlisten?.();
    };
  }, [active, scope, refresh, listenerRevision]);
  const current = active && snapshot?.scope === scope ? snapshot : null;
  useEffect(() => {
    if (!current?.status?.expiresAt) return;
    const remaining = current.status.expiresAt * 1000 - Date.now();
    if (remaining <= 0) return;
    const timer = window.setTimeout(refresh, remaining);
    return () => window.clearTimeout(timer);
  }, [current, refresh]);
  const revision = current?.revision ?? 0;
  const status = current?.status ?? null;
  const isCurrent = useCallback(
    () =>
      active &&
      listening.current &&
      generation.current === revision &&
      admitsCommunity(status),
    [active, revision, status],
  );
  return {
    status,
    error: current?.error ?? false,
    ready: listening.current && admitsCommunity(status),
    revision,
    refresh,
    retry,
    isCurrent,
  };
}

export type SquareEmployeeSessionState = ReturnType<
  typeof useSquareEmployeeSession
>;
