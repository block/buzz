import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  type AsyncStorage,
  type INavigator,
  type IWindow,
  type NavigateParams,
  type NavigateResponse,
  type User,
  UserManager,
  WebStorageStateStore,
} from "oidc-client-ts";

import type { WorkbenchAuthConfig } from "@/features/workbench-auth/workbenchAuthConfig";

export function createWorkbenchCallbackReplayGuard(maxEntries = 16): {
  accept: (callbackUrl: string) => boolean;
} {
  const consumed = new Set<string>();
  return {
    accept(callbackUrl) {
      if (consumed.has(callbackUrl)) return false;
      consumed.add(callbackUrl);
      if (consumed.size > maxEntries) {
        const oldest = consumed.values().next().value;
        if (oldest) consumed.delete(oldest);
      }
      return true;
    },
  };
}

class SystemBrowserNavigator implements INavigator {
  async prepare(): Promise<IWindow> {
    return {
      close: () => undefined,
      navigate: async ({ url }: NavigateParams): Promise<NavigateResponse> => {
        await openUrl(url);
        return new Promise<NavigateResponse>(() => undefined);
      },
    };
  }

  async callback(): Promise<void> {}
}

export function createWorkbenchUserManager(
  config: WorkbenchAuthConfig,
  sessionStore: Storage,
): UserManager {
  const desktopMetadata =
    isTauri() && config.desktopProxyOrigin
      ? createAuthentikDesktopMetadata(config)
      : undefined;
  if (desktopMetadata && config.desktopProxyOrigin)
    installDesktopPocFetch(config.desktopProxyOrigin);
  const settings = {
    authority: config.issuer,
    client_id: config.clientId,
    redirect_uri: config.redirectUri,
    post_logout_redirect_uri: config.postLogoutRedirectUri,
    response_type: "code",
    scope: "openid profile offline_access",
    automaticSilentRenew: true,
    accessTokenExpiringNotificationTimeInSeconds: 120,
    monitorSession: false,
    loadUserInfo: true,
    revokeTokensOnSignout: true,
    ...(desktopMetadata ? { metadata: desktopMetadata } : {}),
    stateStore: new WebStorageStateStore({
      prefix: "buzz.oidc.state.",
      store: sessionStore,
    }),
    userStore: new WebStorageStateStore({
      prefix: "buzz.oidc.user.",
      store: createWorkbenchUserStore(sessionStore),
    }),
  };
  return isTauri()
    ? new UserManager(settings, new SystemBrowserNavigator())
    : new UserManager(settings);
}

type RefreshableUserManager = Pick<UserManager, "getUser" | "signinSilent">;

export function shouldRefreshWorkbenchUser(
  user: Pick<User, "expired" | "expires_at">,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  return (
    user.expired ||
    typeof user.expires_at !== "number" ||
    user.expires_at <= nowSeconds + 120
  );
}

export async function getValidWorkbenchUser(
  manager: RefreshableUserManager,
): Promise<User | null> {
  const current = await manager.getUser();
  if (!current) return null;
  if (!shouldRefreshWorkbenchUser(current)) return current;
  try {
    const refreshed = await manager.signinSilent();
    return refreshed && !refreshed.expired ? refreshed : null;
  } catch {
    return null;
  }
}

function createWorkbenchUserStore(fallback: Storage): AsyncStorage {
  const web: AsyncStorage = {
    get length() {
      return Promise.resolve(fallback.length);
    },
    async clear() {
      fallback.clear();
    },
    async getItem(key) {
      return fallback.getItem(key);
    },
    async key(index) {
      return fallback.key(index);
    },
    async removeItem(key) {
      fallback.removeItem(key);
    },
    async setItem(key, value) {
      fallback.setItem(key, value);
    },
  };
  if (!isTauri()) return web;
  return {
    get length() {
      return invoke<string[]>("workbench_oidc_user_keys")
        .then((keys) => Math.max(keys.length, fallback.length))
        .catch(() => fallback.length);
    },
    async clear() {
      fallback.clear();
      try {
        for (const key of await invoke<string[]>("workbench_oidc_user_keys"))
          await invoke("workbench_oidc_user_delete", { key });
      } catch {
        // Provider revocation keeps a temporarily inaccessible copy unusable.
      }
    },
    async getItem(key) {
      try {
        return (
          (await invoke<string | null>("workbench_oidc_user_load", { key })) ??
          fallback.getItem(key)
        );
      } catch {
        return fallback.getItem(key);
      }
    },
    async key(index) {
      try {
        const secureKey = (await invoke<string[]>("workbench_oidc_user_keys"))[
          index
        ];
        return secureKey ?? fallback.key(index);
      } catch {
        return fallback.key(index);
      }
    },
    async setItem(key, value) {
      try {
        await invoke("workbench_oidc_user_save", { key, value });
        fallback.removeItem(key);
      } catch {
        fallback.setItem(key, value);
      }
    },
    async removeItem(key) {
      fallback.removeItem(key);
      try {
        await invoke("workbench_oidc_user_delete", { key });
      } catch {
        // A locked keyring must not prevent local sign-out.
      }
    },
  };
}

let desktopPocFetchInstalled = false;

function installDesktopPocFetch(proxyOrigin: string): void {
  if (desktopPocFetchInstalled) return;
  desktopPocFetchInstalled = true;
  const nativeFetch = globalThis.fetch.bind(globalThis);
  globalThis.fetch = async (input, init) => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    if (url.origin !== proxyOrigin) return nativeFetch(request);
    const headers = Object.fromEntries(
      [...request.headers.entries()].map(([name, value]) => [
        name.toLowerCase(),
        value,
      ]),
    );
    const result = await invoke<{
      status: number;
      body: string;
      headers: Record<string, string>;
    }>("oidc_poc_proxy", {
      request: {
        path: url.pathname,
        method: request.method,
        body: request.method === "GET" ? null : await request.text(),
        headers,
      },
    });
    return new Response(result.body, {
      status: result.status,
      headers: result.headers,
    });
  };
}

function createAuthentikDesktopMetadata(config: WorkbenchAuthConfig) {
  const issuer = new URL(`${config.issuer.replace(/\/$/, "")}/`);
  const match = issuer.pathname.match(/^\/application\/o\/([^/]+)\/$/);
  if (!match || !config.desktopProxyOrigin)
    throw new Error("Desktop proxy requires an Authentik provider issuer.");
  const providerSlug = match[1];
  const authorization = new URL("/application/o/authorize/", issuer.origin);
  const endSession = new URL(
    `/application/o/${providerSlug}/end-session/`,
    issuer.origin,
  );
  return {
    issuer: issuer.href,
    authorization_endpoint: authorization.href,
    token_endpoint: `${config.desktopProxyOrigin}/_pacioli_oidc/token/`,
    userinfo_endpoint: `${config.desktopProxyOrigin}/_pacioli_oidc/userinfo/`,
    end_session_endpoint: endSession.href,
    jwks_uri: `${config.desktopProxyOrigin}/_pacioli_oidc/${providerSlug}/jwks/`,
  };
}

export function isWorkbenchAuthCallback(
  callbackUrl: string,
  config: WorkbenchAuthConfig,
): boolean {
  return [config.redirectUri, config.postLogoutRedirectUri].some((target) => {
    const expected = new URL(target);
    const actual = new URL(callbackUrl);
    return (
      expected.protocol === actual.protocol &&
      expected.host === actual.host &&
      expected.pathname === actual.pathname
    );
  });
}

export async function processWorkbenchAuthCallback(
  manager: UserManager,
  config: WorkbenchAuthConfig,
  callbackUrl: string,
): Promise<"signin" | "signout" | null> {
  if (!isWorkbenchAuthCallback(callbackUrl, config)) return null;
  const logout = new URL(config.postLogoutRedirectUri);
  const actual = new URL(callbackUrl);
  if (
    logout.protocol === actual.protocol &&
    logout.host === actual.host &&
    logout.pathname === actual.pathname
  ) {
    await manager.signoutCallback(callbackUrl);
    return "signout";
  }
  await manager.signinCallback(callbackUrl);
  if (["http:", "https:"].includes(actual.protocol))
    window.history.replaceState(window.history.state, "", "/");
  return "signin";
}

export async function subscribeToDesktopAuthCallbacks(
  onUrl: (url: string) => void,
): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  for (const url of (await getCurrent()) ?? []) onUrl(url);
  return onOpenUrl((urls) => {
    for (const url of urls) onUrl(url);
  });
}
