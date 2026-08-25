export type WorkbenchAuthConfig = {
  issuer: string;
  clientId: string;
  desktopProxyOrigin?: string;
  redirectUri: string;
  postLogoutRedirectUri: string;
};

export type WorkbenchAuthEnv = {
  VITE_OIDC_ISSUER?: string;
  VITE_OIDC_CLIENT_ID?: string;
  VITE_OIDC_REDIRECT_URI?: string;
  VITE_OIDC_POST_LOGOUT_REDIRECT_URI?: string;
  VITE_OIDC_DESKTOP_PROXY_ORIGIN?: string;
};

type ConfigResult =
  | { config: WorkbenchAuthConfig; error: null }
  | { config: null; error: string | null };

function readAbsoluteUrl(value: string, label: string): URL {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${label} must be an absolute URL.`);
  }
  if (!["https:", "http:", "buzz:", "buzz-dev:"].includes(url.protocol)) {
    throw new Error(
      `${label} must use HTTPS, HTTP, or a Buzz callback scheme.`,
    );
  }
  if (
    url.protocol === "http:" &&
    !["localhost", "127.0.0.1"].includes(url.hostname)
  ) {
    throw new Error(`${label} may use HTTP only on localhost.`);
  }
  if (url.username || url.password || url.hash) {
    throw new Error(`${label} must not contain credentials or a fragment.`);
  }
  return url;
}

export function readWorkbenchAuthConfig(env: WorkbenchAuthEnv): ConfigResult {
  const values = [
    env.VITE_OIDC_ISSUER?.trim(),
    env.VITE_OIDC_CLIENT_ID?.trim(),
    env.VITE_OIDC_REDIRECT_URI?.trim(),
    env.VITE_OIDC_POST_LOGOUT_REDIRECT_URI?.trim(),
  ];
  if (values.every((value) => !value)) return { config: null, error: null };
  if (values.some((value) => !value)) {
    return {
      config: null,
      error:
        "Workbench OIDC is partially configured. Set all four VITE_OIDC_* values.",
    };
  }
  const [issuerValue, clientId, redirectValue, logoutValue] = values as [
    string,
    string,
    string,
    string,
  ];
  if (clientId.length > 256) {
    return { config: null, error: "VITE_OIDC_CLIENT_ID is too long." };
  }
  try {
    const issuer = readAbsoluteUrl(issuerValue, "VITE_OIDC_ISSUER");
    const redirect = readAbsoluteUrl(redirectValue, "VITE_OIDC_REDIRECT_URI");
    const logout = readAbsoluteUrl(
      logoutValue,
      "VITE_OIDC_POST_LOGOUT_REDIRECT_URI",
    );
    if (issuer.protocol === "buzz:" || issuer.search) {
      throw new Error(
        "VITE_OIDC_ISSUER must be an HTTP(S) issuer without a query.",
      );
    }
    return {
      config: {
        issuer: issuer.href.replace(/\/$/, ""),
        clientId,
        ...(env.VITE_OIDC_DESKTOP_PROXY_ORIGIN?.trim()
          ? {
              desktopProxyOrigin: readDesktopProxyOrigin(
                env.VITE_OIDC_DESKTOP_PROXY_ORIGIN,
              ),
            }
          : {}),
        redirectUri: redirect.href,
        postLogoutRedirectUri: logout.href,
      },
      error: null,
    };
  } catch (error) {
    return {
      config: null,
      error:
        error instanceof Error
          ? error.message
          : "Invalid Workbench OIDC configuration.",
    };
  }
}

function readDesktopProxyOrigin(value: string): string {
  const url = new URL(value.trim());
  if (
    url.protocol !== "http:" ||
    !["localhost", "127.0.0.1"].includes(url.hostname) ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  ) {
    throw new Error(
      "VITE_OIDC_DESKTOP_PROXY_ORIGIN must be an HTTP loopback origin.",
    );
  }
  return url.origin;
}

export function getWorkbenchAuthConfig(): ConfigResult {
  return readWorkbenchAuthConfig({
    VITE_OIDC_ISSUER: import.meta.env.VITE_OIDC_ISSUER,
    VITE_OIDC_CLIENT_ID: import.meta.env.VITE_OIDC_CLIENT_ID,
    VITE_OIDC_REDIRECT_URI: import.meta.env.VITE_OIDC_REDIRECT_URI,
    VITE_OIDC_POST_LOGOUT_REDIRECT_URI: import.meta.env
      .VITE_OIDC_POST_LOGOUT_REDIRECT_URI,
    VITE_OIDC_DESKTOP_PROXY_ORIGIN: import.meta.env
      .VITE_OIDC_DESKTOP_PROXY_ORIGIN,
  });
}
