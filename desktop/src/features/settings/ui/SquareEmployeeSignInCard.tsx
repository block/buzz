import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { relayClient } from "@/shared/api/relayClient";
import { Button } from "@/shared/ui/button";
import { useSquareEmployeeSession } from "./useSquareEmployeeSession";

/** Explicitly enabled employee sign-in; the native process retains all session credentials. */
export function SquareEmployeeSignInCard({
  onConnected,
}: {
  onConnected?: () => void;
}) {
  const [available, setAvailable] = useState(false);
  const [pending, setPending] = useState(false);
  const session = useSquareEmployeeSession(
    available,
    "square-employee-settings",
  );
  const signedIn = session.status?.authenticated === true && session.ready;
  const canRenew = session.status?.canRenew === true;
  const canReconnect = signedIn || session.status?.canRetry === true;
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  useEffect(() => {
    let active = true;
    void invoke<boolean>("employee_sign_in_available")
      .then((value) => {
        if (active) setAvailable(value);
      })
      .catch(() => {
        if (active)
          setError("Square sign-in availability could not be checked.");
      });
    return () => {
      active = false;
      generation.current += 1;
    };
  }, []);
  if (!available) return error ? <p role="alert">{error}</p> : null;
  const signIn = async () => {
    const ownGeneration = ++generation.current;
    setPending(true);
    setError(null);
    try {
      await invoke("start_employee_login");
      if (generation.current !== ownGeneration) return;
      session.refresh();
      await relayClient.reconnect();
      if (generation.current === ownGeneration) {
        window.dispatchEvent(new Event("square-employee-auth-changed"));
        onConnected?.();
      }
    } catch (failure) {
      if (generation.current === ownGeneration) setError(String(failure));
    } finally {
      if (generation.current === ownGeneration) setPending(false);
    }
  };
  const cancel = async () => {
    generation.current += 1;
    try {
      await invoke("cancel_builderlab_login");
      setPending(false);
    } catch {
      setError("Could not cancel sign-in. Try again.");
    }
  };
  const signOut = async () => {
    generation.current += 1;
    try {
      await invoke("clear_builderlab_auth");
      relayClient.disconnect();
      session.refresh();
      window.dispatchEvent(new Event("square-employee-auth-changed"));
    } catch {
      setError("Could not sign out. Try again.");
    }
  };
  const reconnect = async () => {
    const ownGeneration = ++generation.current;
    setPending(true);
    setError(null);
    try {
      await invoke("refresh_employee_relay_access");
      if (generation.current !== ownGeneration) return;
      session.refresh();
      await relayClient.reconnect();
      if (generation.current === ownGeneration) {
        window.dispatchEvent(new Event("square-employee-auth-changed"));
        onConnected?.();
      }
    } catch (failure) {
      if (generation.current === ownGeneration) setError(String(failure));
    } finally {
      if (generation.current === ownGeneration) setPending(false);
    }
  };
  return (
    <section
      className="space-y-3 rounded-lg border p-4"
      aria-labelledby="square-employee-sign-in"
    >
      <h2 id="square-employee-sign-in" className="font-semibold">
        Square team member access
      </h2>
      <p className="text-sm text-muted-foreground">
        Sign in with your own Square account to use the access your employer
        prepared.
      </p>
      {signedIn ? (
        <p role="status">
          Square session and your Buzz key are connected. If community admission
          is still pending, retry the connection in a moment.
        </p>
      ) : null}
      {canRenew ? (
        <p role="status">
          {signedIn ? "Renew before access expires to keep working. " : null}
          Continue in your browser with the same Square account and community.
        </p>
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      {session.error ? (
        <>
          <p role="alert">Square session status could not be checked.</p>
          <Button variant="outline" onClick={session.retry}>
            Retry Square access check
          </Button>
        </>
      ) : null}
      <Button
        disabled={pending}
        onClick={() => {
          void signIn();
        }}
      >
        {canRenew ? "Renew Square session" : "Sign in with Square"}
      </Button>
      {pending ? (
        <Button
          variant="outline"
          onClick={() => {
            void cancel();
          }}
        >
          Cancel sign-in
        </Button>
      ) : null}
      {canReconnect ? (
        <Button
          variant="outline"
          disabled={pending}
          onClick={() => {
            void reconnect();
          }}
        >
          Retry community connection
        </Button>
      ) : null}
      {canReconnect || canRenew ? (
        <Button
          variant="outline"
          onClick={() => {
            void signOut();
          }}
        >
          Sign out of Square
        </Button>
      ) : null}
    </section>
  );
}
