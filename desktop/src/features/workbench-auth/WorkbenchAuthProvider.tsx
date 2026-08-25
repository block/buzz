import * as React from "react";
import type { UserManager } from "oidc-client-ts";
import {
  bindCurrentDevice as bindGatewayDevice,
  getBusinessAuthGatewayUrl,
  logoutWorkbenchSession,
  readGatewayState,
  type WorkbenchAuthState,
} from "@/features/workbench-auth/businessAuthGateway";

import {
  createWorkbenchCallbackReplayGuard,
  createWorkbenchUserManager,
  getValidWorkbenchUser,
  processWorkbenchAuthCallback,
  subscribeToDesktopAuthCallbacks,
} from "@/features/workbench-auth/workbenchAuthClient";
import {
  getWorkbenchAuthConfig,
  type WorkbenchAuthConfig,
} from "@/features/workbench-auth/workbenchAuthConfig";
import { Button } from "@/shared/ui/button";

type AuthPhase =
  | "unconnected"
  | "checking"
  | "signing-in"
  | "authenticated"
  | "expired"
  | "failed";

type AuthIdentity = { subject: string; displayName: string };
type GroupClaimStatus = "verified" | "missing" | null;

type WorkbenchAuthContextValue = {
  config: WorkbenchAuthConfig | null;
  error: string | null;
  identity: AuthIdentity | null;
  groupClaimStatus: GroupClaimStatus;
  gatewayUrl: string | null;
  gatewayState: WorkbenchAuthState;
  phase: AuthPhase;
  bindCurrentDevice: () => Promise<void>;
  getAccessToken: () => Promise<string | null>;
  signIn: () => Promise<void>;
  signOutWorkbench: () => Promise<void>;
  signOut: () => Promise<void>;
};

const WorkbenchAuthContext =
  React.createContext<WorkbenchAuthContextValue | null>(null);

function identityFromProfile(profile: Record<string, unknown>): AuthIdentity {
  const subject = typeof profile.sub === "string" ? profile.sub : "";
  const displayName = [profile.name, profile.preferred_username, subject].find(
    (value): value is string => typeof value === "string" && value.length > 0,
  );
  return { subject, displayName: displayName ?? "Authenticated user" };
}

function groupClaimStatusFromProfile(
  profile: Record<string, unknown>,
): Exclude<GroupClaimStatus, null> {
  const groups = Array.isArray(profile.groups)
    ? profile.groups.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  return ["bizfin-finance", "bizfin-business"].every((group) =>
    groups.includes(group),
  )
    ? "verified"
    : "missing";
}

export function WorkbenchAuthProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const result = React.useMemo(getWorkbenchAuthConfig, []);
  const manager = React.useMemo<UserManager | null>(
    () =>
      result.config && typeof window !== "undefined"
        ? createWorkbenchUserManager(result.config, window.sessionStorage)
        : null,
    [result.config],
  );
  const [phase, setPhase] = React.useState<AuthPhase>(
    result.config ? "checking" : result.error ? "failed" : "unconnected",
  );
  const [error, setError] = React.useState<string | null>(result.error);
  const [identity, setIdentity] = React.useState<AuthIdentity | null>(null);
  const [groupClaimStatus, setGroupClaimStatus] =
    React.useState<GroupClaimStatus>(null);
  const gatewayUrl = React.useMemo(getBusinessAuthGatewayUrl, []);
  const [gatewayState, setGatewayState] = React.useState<WorkbenchAuthState>({
    status: result.config ? "initializing" : "unauthenticated",
  });
  const callbackReplayGuard = React.useRef(
    createWorkbenchCallbackReplayGuard(),
  );

  const syncGateway = React.useCallback(
    async (accessToken: string | undefined) => {
      if (!gatewayUrl || !accessToken) {
        setGatewayState({ status: "unauthenticated" });
        return;
      }
      try {
        setGatewayState(await readGatewayState(gatewayUrl, accessToken));
      } catch (cause) {
        setGatewayState({
          status: "error",
          error:
            cause instanceof Error
              ? cause.message
              : "Business identity check failed.",
        });
      }
    },
    [gatewayUrl],
  );

  const consumeCallback = React.useCallback(
    async (url: string) => {
      if (!manager || !result.config) return;
      if (!callbackReplayGuard.current.accept(url)) return;
      try {
        const processed = await processWorkbenchAuthCallback(
          manager,
          result.config,
          url,
        );
        if (!processed) return;
        const user = await manager.getUser();
        setIdentity(
          user && !user.expired ? identityFromProfile(user.profile) : null,
        );
        setGroupClaimStatus(
          user && !user.expired
            ? groupClaimStatusFromProfile(user.profile)
            : null,
        );
        setPhase(user && !user.expired ? "authenticated" : "unconnected");
        await syncGateway(
          user && !user.expired ? user.access_token : undefined,
        );
        setError(null);
      } catch (cause) {
        setPhase("failed");
        setError(
          cause instanceof Error ? cause.message : "OIDC callback failed.",
        );
      }
    },
    [manager, result.config, syncGateway],
  );

  React.useEffect(() => {
    if (!manager || !result.config) return;
    let disposed = false;
    let cleanup: () => void = () => undefined;
    void (async () => {
      const currentUrl = window.location.href;
      if (currentUrl.includes("code=") || currentUrl.includes("error=")) {
        await consumeCallback(currentUrl);
      } else {
        const stored = await manager.getUser();
        const user = stored ? await getValidWorkbenchUser(manager) : null;
        if (disposed) return;
        setIdentity(
          user && !user.expired ? identityFromProfile(user.profile) : null,
        );
        setGroupClaimStatus(
          user && !user.expired
            ? groupClaimStatusFromProfile(user.profile)
            : null,
        );
        setPhase(user ? "authenticated" : stored ? "expired" : "unconnected");
        await syncGateway(
          user && !user.expired ? user.access_token : undefined,
        );
      }
      const unsubscribe = await subscribeToDesktopAuthCallbacks((url) => {
        void consumeCallback(url);
      });
      if (disposed) unsubscribe();
      else cleanup = unsubscribe;
    })().catch((cause) => {
      if (!disposed) {
        setPhase("failed");
        setError(
          cause instanceof Error
            ? cause.message
            : "OIDC initialization failed.",
        );
      }
    });
    return () => {
      disposed = true;
      cleanup();
    };
  }, [consumeCallback, manager, result.config, syncGateway]);

  React.useEffect(() => {
    if (!manager) return;
    const userLoaded = (user: Awaited<ReturnType<UserManager["getUser"]>>) => {
      if (!user || user.expired) return;
      setIdentity(identityFromProfile(user.profile));
      setGroupClaimStatus(groupClaimStatusFromProfile(user.profile));
      setPhase("authenticated");
      setError(null);
      void syncGateway(user.access_token);
    };
    const silentRenewError = () => {
      void manager.getUser().then((user) => {
        if (!user || user.expired) {
          setIdentity(null);
          setGroupClaimStatus(null);
          setGatewayState({ status: "unauthenticated" });
          setPhase("expired");
        }
      });
    };
    manager.events.addUserLoaded(userLoaded);
    manager.events.addSilentRenewError(silentRenewError);
    return () => {
      manager.events.removeUserLoaded(userLoaded);
      manager.events.removeSilentRenewError(silentRenewError);
    };
  }, [manager, syncGateway]);

  const value = React.useMemo<WorkbenchAuthContextValue>(
    () => ({
      config: result.config,
      error,
      gatewayUrl,
      gatewayState,
      identity,
      groupClaimStatus,
      phase,
      getAccessToken: async () => {
        if (!manager) return null;
        const user = await getValidWorkbenchUser(manager);
        if (!user) {
          setIdentity(null);
          setGroupClaimStatus(null);
          setGatewayState({ status: "unauthenticated" });
          setPhase("expired");
          return null;
        }
        setIdentity(identityFromProfile(user.profile));
        setGroupClaimStatus(groupClaimStatusFromProfile(user.profile));
        setPhase("authenticated");
        await syncGateway(user.access_token);
        return user.access_token;
      },
      bindCurrentDevice: async () => {
        if (!manager || !gatewayUrl) return;
        const user = await manager.getUser();
        if (!user || user.expired) {
          setGatewayState({ status: "unauthenticated" });
          return;
        }
        setGatewayState({ status: "authenticating" });
        try {
          setGatewayState(
            await bindGatewayDevice(gatewayUrl, user.access_token),
          );
        } catch (cause) {
          setGatewayState({
            status: "error",
            error:
              cause instanceof Error ? cause.message : "Device binding failed.",
          });
        }
      },
      signIn: async () => {
        if (!manager) return;
        setPhase("signing-in");
        setError(null);
        try {
          await manager.signinRedirect();
        } catch (cause) {
          setPhase("failed");
          setError(cause instanceof Error ? cause.message : "Sign-in failed.");
        }
      },
      signOut: async () => {
        if (!manager) return;
        setError(null);
        try {
          const user = await manager.getUser();
          if (gatewayUrl && user && !user.expired) {
            await logoutWorkbenchSession(gatewayUrl, user.access_token, true);
          }
          await manager.signoutRedirect();
        } catch (cause) {
          setPhase("failed");
          setError(cause instanceof Error ? cause.message : "Sign-out failed.");
        }
      },
      signOutWorkbench: async () => {
        if (!manager) return;
        setError(null);
        try {
          const user = await manager.getUser();
          if (gatewayUrl && user && !user.expired)
            await logoutWorkbenchSession(gatewayUrl, user.access_token, false);
          await manager.removeUser();
          setIdentity(null);
          setGroupClaimStatus(null);
          setGatewayState({ status: "unauthenticated" });
          setPhase("unconnected");
        } catch (cause) {
          setPhase("failed");
          setError(
            cause instanceof Error
              ? cause.message
              : "Workbench sign-out failed.",
          );
        }
      },
    }),
    [
      error,
      gatewayState,
      gatewayUrl,
      groupClaimStatus,
      identity,
      manager,
      phase,
      result.config,
      syncGateway,
    ],
  );

  return (
    <WorkbenchAuthContext.Provider value={value}>
      {children}
    </WorkbenchAuthContext.Provider>
  );
}

export function WorkbenchAuthGate({ children }: { children: React.ReactNode }) {
  const auth = useWorkbenchAuth();
  if (!auth.config) return <>{children}</>;
  if (auth.phase === "authenticated") return <>{children}</>;
  return (
    <main
      className="grid min-h-dvh place-items-center bg-background p-6"
      data-testid="workbench-auth-gate"
    >
      <section className="w-full max-w-md rounded-xl border bg-card p-8 shadow-sm">
        <p className="mb-2 text-sm font-medium text-muted-foreground">
          Pacioli AI · Authentik POC
        </p>
        <h1 className="text-2xl font-semibold">Sign in to Workbench</h1>
        <p className="mt-3 text-sm text-muted-foreground">
          Authentication opens in a top-level browser context. Tokens are never
          sent to Business Dock.
        </p>
        {auth.error ? (
          <p className="mt-4 text-sm text-destructive">{auth.error}</p>
        ) : null}
        <Button
          className="mt-6 w-full"
          disabled={auth.phase === "checking" || auth.phase === "signing-in"}
          onClick={() => void auth.signIn()}
        >
          {auth.phase === "checking"
            ? "Checking session…"
            : auth.phase === "signing-in"
              ? "Continue in browser…"
              : "Sign in with Authentik"}
        </Button>
      </section>
    </main>
  );
}

export function useWorkbenchAuth(): WorkbenchAuthContextValue {
  const value = React.useContext(WorkbenchAuthContext);
  if (!value)
    throw new Error(
      "useWorkbenchAuth must be used within WorkbenchAuthProvider",
    );
  return value;
}
