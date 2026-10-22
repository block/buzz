import type { ReactNode } from "react";
import { Button } from "@/shared/ui/button";
import { SquareEmployeeSignInCard } from "./SquareEmployeeSignInCard";
import {
  useSquareEmployeeSession,
  type SquareEmployeeSessionState,
} from "./useSquareEmployeeSession";

/** Protects relay content while keeping local community and identity recovery available. */
export function SquareEmployeeSessionGate({
  active,
  scope,
  session,
  recovery,
  children,
}: {
  active: boolean;
  scope: string;
  session?: SquareEmployeeSessionState;
  recovery?: ReactNode;
  children: ReactNode;
}) {
  if (!active) return children;
  if (session)
    return (
      <SessionGateContent session={session} recovery={recovery}>
        {children}
      </SessionGateContent>
    );
  return (
    <ActiveSessionGate key={scope} scope={scope} recovery={recovery}>
      {children}
    </ActiveSessionGate>
  );
}

function ActiveSessionGate({
  scope,
  recovery,
  children,
}: {
  scope: string;
  recovery?: ReactNode;
  children: ReactNode;
}) {
  const session = useSquareEmployeeSession(true, scope);
  return (
    <SessionGateContent session={session} recovery={recovery}>
      {children}
    </SessionGateContent>
  );
}

function SessionGateContent({
  session,
  recovery,
  children,
}: {
  session: SquareEmployeeSessionState;
  recovery?: ReactNode;
  children: ReactNode;
}) {
  if (session.ready) return children;
  return (
    <main className="flex min-h-dvh items-center justify-center p-6">
      <div className="max-w-md space-y-4">
        {session.error ? (
          <>
            <p role="alert">Square access could not be checked.</p>
            <Button onClick={session.retry}>Retry access check</Button>
          </>
        ) : null}
        {session.status?.required ? (
          <SquareEmployeeSignInCard onConnected={session.refresh} />
        ) : (
          <p role="status">Checking community access…</p>
        )}
        {recovery}
      </div>
    </main>
  );
}
