import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { router } from "@/app/router";
import { KnownAgentPubkeysProvider } from "@/features/agents/useKnownAgentPubkeys";
import {
  FixedCommunityProvider,
  useCommunities,
} from "@/features/communities/useCommunities";
import type { Community } from "@/features/communities/types";
import { initDraftStore } from "@/features/messages/lib/useDrafts";
import { LandingBees } from "@/features/onboarding/ui/LandingBees";
import {
  ONBOARDING_LANDING_CTA_CLASS,
  ONBOARDING_SECONDARY_CTA_CLASS,
} from "@/features/onboarding/ui/OnboardingChrome";
import { createBuzzQueryClient } from "@/shared/api/queryClient";
import {
  nativeIdentity,
  revokeNativeIdentity,
  type NativeIdentityStatus,
} from "@/shared/api/nativeIdentitySession";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";

/** Remote custody deliberately bypasses machine/nsec onboarding and backup UI.
 * Each webview realm has one auth generation. Logout/replacement revokes native
 * effects first, clears singleton state, then reloads to mount a fresh realm.
 */
export function RemoteIdentityApp() {
  const identity = nativeIdentity();
  const [client] = useState(createBuzzQueryClient);
  const communities = useCommunities();
  const [ready, setReady] = useState(false);
  const [workspace, setWorkspace] = useState<Community | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [degraded, setDegraded] = useState(
    () => sessionStorage.getItem("buzz-remote-archive-degraded") !== null,
  );
  const [relayUrl, setRelayUrl] = useState(
    "wss://buzz.test.blockstaging.build",
  );
  const generation = identity?.generation;
  const pubkey = identity?.publicIdentity;
  const authenticated = identity?.authState === "authenticated" && !!pubkey;

  useEffect(() => {
    let alive = true;
    const subscription = listen("archive-sync-degraded", () => {
      sessionStorage.setItem("buzz-remote-archive-degraded", "true");
      if (alive) setDegraded(true);
    });
    return () => {
      alive = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (!authenticated) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const status = await invoke<NativeIdentityStatus>(
          "get_native_identity_status",
        );
        if (stopped) return;
        if (
          status.generation !== generation ||
          status.authState !== "authenticated"
        ) {
          revokeNativeIdentity();
          setReady(false);
          client.clear();
          window.location.reload();
          return;
        }
      } catch (cause) {
        if (!stopped) setError(String(cause));
      }
      if (!stopped) timer = setTimeout(() => void poll(), 1000);
    }
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [authenticated, generation, client]);

  async function login() {
    setBusy(true);
    setError(null);
    try {
      await invoke("start_builderlab_login");
      window.location.reload();
    } catch (cause) {
      setError(String(cause));
      setBusy(false);
    }
  }
  async function activate() {
    if (!pubkey || generation === undefined) return;
    setBusy(true);
    setError(null);
    try {
      const normalized = relayUrl.trim().replace(/\/$/, "");
      await invoke("activate_remote_workspace", {
        relayUrl: normalized,
        expectedGeneration: generation,
      });
      const status = await invoke<NativeIdentityStatus>(
        "get_native_identity_status",
      );
      if (status.generation !== generation || !status.workspaceActive)
        throw new Error("Session changed during activation");
      const existing = communities.communities.find(
        (community) => community.relayUrl === normalized,
      );
      // Do not adopt a saved local nsec/token or overwrite its custody metadata.
      setWorkspace({
        id: existing?.id ?? crypto.randomUUID(),
        name: existing?.name ?? "Staging",
        relayUrl: normalized,
        pubkey,
        addedAt: new Date().toISOString(),
      });
      initDraftStore(pubkey, normalized);
      setReady(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  }

  if (ready && workspace)
    return (
      <QueryClientProvider client={client}>
        <div className="pointer-events-none fixed right-4 top-12 z-50 flex max-w-sm flex-col gap-2">
          {degraded && (
            <p
              className="pointer-events-auto rounded-xl border border-destructive/30 bg-background px-4 py-3 text-sm shadow-lg"
              role="alert"
            >
              Observer archive is degraded. Some frames may be missing;
              reconnecting does not recover ephemeral gaps.
            </p>
          )}
          {error && (
            <p
              role="alert"
              className="pointer-events-auto rounded-xl border bg-background px-4 py-3 text-sm text-destructive shadow-lg"
            >
              {error}
            </p>
          )}
        </div>
        <FixedCommunityProvider community={workspace}>
          <KnownAgentPubkeysProvider>
            <RouterProvider router={router} />
          </KnownAgentPubkeysProvider>
        </FixedCommunityProvider>
      </QueryClientProvider>
    );
  return (
    <main className="buzz-onboarding-neutral-theme buzz-startup-shell buzz-onboarding-welcome flex max-h-dvh items-start justify-center overflow-x-hidden overflow-y-auto px-4 py-8 text-foreground">
      <StartupWindowDragRegion />
      <LandingBees />
      <div className="relative my-auto flex w-full max-w-[720px] flex-col items-center text-center">
        <h1 className="w-full max-w-[600px]">
          <img alt="Buzz" className="w-full" src="/landing/buzz-wordmark.png" />
        </h1>
        <p className="mt-2 max-w-[560px] text-2xl font-normal leading-tight">
          Your people, your agents, your projects —<br />
          all in one place.
        </p>
        <div className="mt-10 flex w-full max-w-sm flex-col items-center gap-4">
          {error && (
            <p
              role="alert"
              className="w-full break-words rounded-xl border border-destructive/30 bg-background/80 p-3 text-sm text-destructive"
            >
              {error}
            </p>
          )}
          {authenticated ? (
            <form
              className="flex w-full flex-col items-center gap-4"
              onSubmit={(event) => {
                event.preventDefault();
                if (!busy && relayUrl.trim()) void activate();
              }}
            >
              <label className="flex w-full flex-col gap-2 text-left text-sm">
                Workspace relay
                <input
                  className="w-full rounded-xl border border-foreground/20 bg-background/60 px-4 py-3 text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-foreground"
                  value={relayUrl}
                  onChange={(event) => setRelayUrl(event.target.value)}
                  placeholder="wss://buzz.test.blockstaging.build"
                  disabled={busy}
                  autoCapitalize="none"
                  spellCheck={false}
                />
              </label>
              <Button
                className={ONBOARDING_LANDING_CTA_CLASS}
                type="submit"
                disabled={busy || !relayUrl.trim()}
              >
                {busy ? "Opening workspace…" : "Open workspace"}
              </Button>
            </form>
          ) : (
            <>
              <Button
                className={ONBOARDING_LANDING_CTA_CLASS}
                disabled={busy}
                onClick={() => void login()}
              >
                {busy ? "Waiting for browser sign-in…" : "Sign in with Block"}
              </Button>
              {busy && (
                <Button
                  className={ONBOARDING_SECONDARY_CTA_CLASS}
                  variant="ghost"
                  onClick={() => {
                    void invoke("cancel_builderlab_login").catch((cause) =>
                      setError(String(cause)),
                    );
                  }}
                >
                  Cancel sign-in
                </Button>
              )}
            </>
          )}
          <p className="text-sm text-foreground/70">
            {authenticated
              ? "Staging workspace · remote signing preview"
              : "Continue in your browser with your Block account."}
            <br />
            No private key to set up on this device.
          </p>
        </div>
      </div>
    </main>
  );
}
