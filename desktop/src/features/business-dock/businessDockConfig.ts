export type BusinessDockConfig = {
  homeUrl: string;
  origin: string;
};

export type BusinessDockConfigResult =
  | { config: BusinessDockConfig; error: null }
  | { config: null; error: string };

export type BusinessDockEnv = {
  VITE_BUSINESS_APP_ORIGIN?: string;
  VITE_BUSINESS_APP_URL?: string;
};

const SUPPORTED_PROTOCOLS = new Set(["http:", "https:"]);

function parseHttpUrl(value: string, base?: string): URL | null {
  try {
    const url = base ? new URL(value, base) : new URL(value);
    return SUPPORTED_PROTOCOLS.has(url.protocol) ? url : null;
  } catch {
    return null;
  }
}

export function readBusinessDockConfig(
  env: BusinessDockEnv,
): BusinessDockConfigResult {
  const rawOrigin = env.VITE_BUSINESS_APP_ORIGIN?.trim();
  const rawHomeUrl = env.VITE_BUSINESS_APP_URL?.trim();

  if (!rawOrigin || !rawHomeUrl) {
    return {
      config: null,
      error: "Business system is not configured.",
    };
  }

  const originUrl = parseHttpUrl(rawOrigin);
  if (
    !originUrl ||
    originUrl.username ||
    originUrl.password ||
    originUrl.pathname !== "/" ||
    originUrl.search ||
    originUrl.hash
  ) {
    return {
      config: null,
      error: "Business system origin is invalid.",
    };
  }

  const homeUrl = parseHttpUrl(rawHomeUrl, `${originUrl.origin}/`);
  if (!homeUrl || homeUrl.origin !== originUrl.origin) {
    return {
      config: null,
      error: "Business system URL must use the configured origin.",
    };
  }

  return {
    config: {
      homeUrl: homeUrl.href,
      origin: originUrl.origin,
    },
    error: null,
  };
}

export function resolveAllowedBusinessUrl(
  value: string,
  config: BusinessDockConfig,
  baseUrl = config.homeUrl,
): string | null {
  const url = parseHttpUrl(value, baseUrl);
  return url?.origin === config.origin ? url.href : null;
}

export function isAllowedBusinessUrl(
  value: string,
  config: BusinessDockConfig,
  baseUrl = config.homeUrl,
): boolean {
  return resolveAllowedBusinessUrl(value, config, baseUrl) !== null;
}

export function getBusinessDockConfig(): BusinessDockConfigResult {
  // Keep these as direct import.meta.env reads so Vite can replace them at
  // build time. Reading the env object through an alias is not statically
  // transformed in production builds.
  return readBusinessDockConfig({
    VITE_BUSINESS_APP_ORIGIN: import.meta.env.VITE_BUSINESS_APP_ORIGIN,
    VITE_BUSINESS_APP_URL: import.meta.env.VITE_BUSINESS_APP_URL,
  });
}
