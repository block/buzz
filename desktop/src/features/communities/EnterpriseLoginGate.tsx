import { useEffect, useState, useRef, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useCommunities } from "./useCommunities";
import { markCommunityOnboardingComplete } from "@/features/onboarding/communityOnboarding";

type Identity = { pubkey: string; relayWsUrl: string; relayHttpUrl: string };
type Status = { enabled: boolean; identity: Identity | null };

/** Release-selected login gate; never turns a failed corporate login into local-key onboarding. */
export function EnterpriseLoginGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<Status | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const { addCommunity, switchCommunity } = useCommunities();

  function accept(identity: Identity) {
    const id = addCommunity({
      id: `enterprise-${identity.pubkey}`,
      name: "Work",
      relayUrl: identity.relayWsUrl,
      pubkey: identity.pubkey,
      addedAt: new Date().toISOString(),
    });
    switchCommunity(id);
    markCommunityOnboardingComplete(identity.pubkey, identity.relayWsUrl);
    localStorage.setItem(
      `buzz-machine-onboarding-complete.v2:${identity.pubkey}`,
      "true",
    );
    setStatus({ enabled: true, identity });
  }

  const acceptRef = useRef(accept);
  acceptRef.current = accept;

  useEffect(() => {
    let active = true;
    invoke<Status>("enterprise_status")
      .then((next) => {
        if (!active) return;
        if (next.identity) acceptRef.current(next.identity);
        else setStatus(next);
      })
      .catch(() => {
        if (active) {
          setStatus({ enabled: true, identity: null });
          setError("Your work session could not be restored. Sign in again.");
        }
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!status?.enabled || !status.identity) return;
    let active = true;
    const timer = window.setInterval(() => {
      void invoke<Status>("enterprise_status")
        .then((next) => {
          if (active && !next.identity) {
            setStatus(next);
            setError("Your work session expired. Sign in again.");
          }
        })
        .catch(() => {
          if (active) {
            setStatus({ enabled: true, identity: null });
            setError(
              "Your work session could not be refreshed. Sign in again.",
            );
          }
        });
    }, 60_000);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [status?.enabled, status?.identity]);

  if (status?.enabled === false) return children;
  if (status?.identity)
    return (
      <>
        {children}
        <button
          type="button"
          className="fixed bottom-2 right-2 z-50 rounded bg-background px-3 py-1 text-xs text-muted-foreground shadow"
          onClick={async () => {
            try {
              await invoke("enterprise_logout");
              setStatus({ enabled: true, identity: null });
            } catch {
              setError(
                "Sign-out could not remove saved credentials. Try again.",
              );
              setStatus({ enabled: true, identity: null });
            }
          }}
        >
          Sign out of work account
        </button>
      </>
    );

  return (
    <main className="flex h-screen flex-col items-center justify-center gap-4 bg-background p-8 text-foreground">
      <h1 className="text-xl font-semibold">Sign in to Buzz for work</h1>
      <p className="text-sm">
        Your organization manages your identity and community.
      </p>
      {error ? (
        <p role="alert" className="text-sm">
          {error}
        </p>
      ) : null}
      <button
        type="button"
        className="rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-50"
        disabled={!status || busy}
        onClick={async () => {
          setBusy(true);
          setError(null);
          try {
            accept(await invoke<Identity>("enterprise_login"));
          } catch {
            setError("Sign-in did not complete. Try again.");
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy
          ? "Waiting for corporate sign-in…"
          : "Sign in with corporate account"}
      </button>
    </main>
  );
}
