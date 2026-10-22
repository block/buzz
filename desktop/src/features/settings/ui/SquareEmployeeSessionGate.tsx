import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/shared/ui/button";
import { SquareEmployeeSignInCard } from "./SquareEmployeeSignInCard";

type Status = {
  required: boolean;
  authenticated: boolean;
  expiresAt: number | null;
};

/** Shows employee login before relay-dependent onboarding can hide its recovery action. */
export function SquareEmployeeSessionGate({
  active,
  scope,
  children,
}: {
  active: boolean;
  scope: string;
  children: ReactNode;
}) {
  if (!active) return children;
  return <ActiveSessionGate key={scope}>{children}</ActiveSessionGate>;
}

function ActiveSessionGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState(false);
  const generation = useRef(0);
  const refresh = useCallback(() => {
    const ownGeneration = ++generation.current;
    void invoke<Status>("employee_session_status")
      .then((value) => {
        if (generation.current !== ownGeneration) return;
        setStatus(value);
        setError(false);
      })
      .catch(() => {
        if (generation.current !== ownGeneration) return;
        setStatus(null);
        setError(true);
      });
  }, []);
  useEffect(() => {
    refresh();
    window.addEventListener("square-employee-auth-changed", refresh);
    return () => {
      generation.current += 1;
      window.removeEventListener("square-employee-auth-changed", refresh);
    };
  }, [refresh]);
  useEffect(() => {
    if (!status?.expiresAt) return;
    const timer = window.setTimeout(
      () => {
        setStatus(null);
        refresh();
      },
      Math.max(0, status.expiresAt * 1000 - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [status, refresh]);
  if (
    status &&
    (!status.required ||
      (status.authenticated &&
        status.expiresAt !== null &&
        status.expiresAt * 1000 > Date.now()))
  )
    return children;
  return (
    <main className="flex min-h-dvh items-center justify-center p-6">
      <div className="max-w-md space-y-4">
        {error ? (
          <>
            <p role="alert">Square access could not be checked.</p>
            <Button onClick={refresh}>Retry access check</Button>
          </>
        ) : null}
        {status?.required ? (
          <SquareEmployeeSignInCard onConnected={refresh} />
        ) : (
          <p role="status">Checking community access…</p>
        )}
      </div>
    </main>
  );
}
