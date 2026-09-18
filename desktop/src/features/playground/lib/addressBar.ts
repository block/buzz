import { isAllowedPlaygroundUrl } from "./url.ts";

export type LockedPlaygroundUrl = {
  prefix: string;
  suffix: string;
};

/**
 * Lock the start URL through its last path slash. `https://host` and
 * `https://host/` both lock `https://host/`. A path like `/foo/bar` locks
 * `https://host/foo/` and leaves `bar` (plus query/hash) editable.
 */
export function splitLockedPlaygroundUrl(
  startUrl: string,
): LockedPlaygroundUrl {
  try {
    const url = new URL(startUrl);
    const path = url.pathname || "/";
    const slash = path.lastIndexOf("/");
    const pathPrefix = slash >= 0 ? path.slice(0, slash + 1) : "/";
    return {
      prefix: `${url.origin}${pathPrefix}`,
      suffix: `${path.slice(slash + 1)}${url.search}${url.hash}`,
    };
  } catch {
    const slash = startUrl.lastIndexOf("/");
    if (slash < 0) {
      return { prefix: startUrl, suffix: "" };
    }
    return {
      prefix: startUrl.slice(0, slash + 1),
      suffix: startUrl.slice(slash + 1),
    };
  }
}

/** True when `url` is still under the locked prefix (incl. bare origin). */
export function isUrlUnderPlaygroundLock(prefix: string, url: string): boolean {
  if (url.startsWith(prefix)) return true;
  // `https://host` is under lock `https://host/`.
  return prefix.endsWith("/") && url === prefix.slice(0, -1);
}

export function suffixFromCurrentUrl(
  startUrl: string,
  currentUrl: string,
): string {
  const { prefix } = splitLockedPlaygroundUrl(startUrl);
  if (currentUrl.startsWith(prefix)) {
    return currentUrl.slice(prefix.length);
  }
  if (prefix.endsWith("/") && currentUrl === prefix.slice(0, -1)) {
    return "";
  }
  return splitLockedPlaygroundUrl(currentUrl).suffix;
}

export function joinPlaygroundUrl(prefix: string, suffix: string): string {
  return `${prefix}${suffix}`;
}

export function playgroundAddressNavigation(
  startUrl: string,
  suffix: string,
): { ok: true; url: string } | { ok: false; message: string } {
  const { prefix } = splitLockedPlaygroundUrl(startUrl);
  const url = joinPlaygroundUrl(prefix, suffix);
  if (!isAllowedPlaygroundUrl(url)) {
    return {
      ok: false,
      message: "Playground URL must stay https and off debug ports.",
    };
  }
  return { ok: true, url };
}

export type PlaygroundAddressDisplay =
  | { mode: "locked"; prefix: string; suffix: string }
  | { mode: "full"; url: string };

/**
 * Address chrome display: locked host+path + editable suffix while still under
 * the start-URL lock, otherwise the full current URL once (no locked-prefix +
 * foreign-host double display after redirects).
 */
export function playgroundAddressDisplay(
  startUrl: string,
  currentUrl: string,
): PlaygroundAddressDisplay {
  const locked = splitLockedPlaygroundUrl(startUrl);
  const url = currentUrl || startUrl;
  if (isUrlUnderPlaygroundLock(locked.prefix, url)) {
    return {
      mode: "locked",
      prefix: locked.prefix,
      suffix: suffixFromCurrentUrl(startUrl, url),
    };
  }
  return { mode: "full", url };
}
