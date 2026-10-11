import * as React from "react";

import { looksLikeRelayMediaUrl } from "./mediaUrl";

export type RelayMediaState = "ready" | "idle" | "loading" | "error";

export type RelayMediaResult = {
  /** URL to put in `<img src>`; undefined until `state === "ready"`. */
  src: string | undefined;
  state: RelayMediaState;
  error: string | null;
  retry: () => void;
};

const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";

type WebMedia = typeof import("@/platform/web/relayMedia");
let webMediaPromise: Promise<WebMedia> | null = null;
function loadWebMedia(): Promise<WebMedia> {
  // Constant-folded away in the desktop build (VITE_BUZZ_PLATFORM is only defined for the web build).
  webMediaPromise ??= IS_WEB
    ? import("@/platform/web/relayMedia")
    : Promise.reject(new Error("relay media fetch is web-only"));
  return webMediaPromise;
}

/**
 * Desktop: identity (the localhost proxy already authenticates).
 * Web: relay media URLs need an Authorization header a plain <img> cannot send, so fetch the bytes (shared signed token,
 * same-origin /__relay proxy, bounded concurrency, cached) and hand back a blob: URL. Starts only once `visible` is true.
 */
export function useRelayMediaSrc(
  url: string | undefined,
  visible = true,
): RelayMediaResult {
  const needsFetch = IS_WEB && looksLikeRelayMediaUrl(url);
  const [attempt, setAttempt] = React.useState(0);
  const [result, setResult] = React.useState<{
    url: string;
    attempt: number;
    state: "ready" | "error" | "loading";
    src?: string;
    error?: string;
  } | null>(null);

  React.useEffect(() => {
    if (!needsFetch || !url || !visible) return;
    let cancelled = false;
    setResult({ url, attempt, state: "loading" });
    loadWebMedia()
      .then(async (m) => {
        if (!m.isRelayMediaUrl(url)) return url; // external host: load directly
        return m.getRelayMediaBlobUrl(url);
      })
      .then((src) => {
        if (!cancelled) setResult({ url, attempt, state: "ready", src });
      })
      .catch((e) => {
        if (!cancelled)
          setResult({
            url,
            attempt,
            state: "error",
            error: e instanceof Error ? e.message : String(e),
          });
      });
    return () => {
      cancelled = true;
    };
  }, [needsFetch, url, visible, attempt]);

  const retry = React.useCallback(() => setAttempt((n) => n + 1), []);

  if (!needsFetch) return { src: url, state: "ready", error: null, retry };
  if (!visible && result?.url !== url)
    return { src: undefined, state: "idle", error: null, retry };
  if (result && result.url === url && result.attempt === attempt) {
    return {
      src: result.state === "ready" ? result.src : undefined,
      state: result.state,
      error: result.error ?? null,
      retry,
    };
  }
  return { src: undefined, state: "loading", error: null, retry };
}

/** Becomes true (and stays true) once the element is within `margin` px of the viewport. */
export function useNearViewport(margin = 400) {
  const [near, setNear] = React.useState(
    !IS_WEB || typeof IntersectionObserver === "undefined",
  );
  const observerRef = React.useRef<IntersectionObserver | null>(null);
  const ref = React.useCallback(
    (el: Element | null) => {
      observerRef.current?.disconnect();
      observerRef.current = null;
      if (!el || near) return;
      const io = new IntersectionObserver(
        (entries) => {
          if (entries.some((e) => e.isIntersecting)) {
            setNear(true);
            io.disconnect();
          }
        },
        { rootMargin: `${margin}px` },
      );
      io.observe(el);
      observerRef.current = io;
    },
    [near, margin],
  );
  return { near, ref };
}
