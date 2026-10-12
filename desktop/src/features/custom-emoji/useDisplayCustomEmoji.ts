import * as React from "react";

import { looksLikeRelayMediaUrl } from "@/shared/lib/mediaUrl";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

const IS_WEB = import.meta.env?.VITE_BUZZ_PLATFORM === "web";

/**
 * For surfaces that need a plain URL string (the emoji-mart picker): on web, swap each relay-hosted emoji URL for an
 * authenticated blob: URL once fetched (emoji without one yet are left out until it arrives). Desktop: unchanged.
 * Display only: never publish these URLs.
 */
export function useDisplayCustomEmoji(emoji: CustomEmoji[]): CustomEmoji[] {
  const [resolved, setResolved] = React.useState<Record<string, string>>({});
  React.useEffect(() => {
    if (!IS_WEB || emoji.length === 0) return;
    let dead = false;
    void import("@/platform/web/relayMedia").then((m) => {
      for (const e of emoji) {
        if (!m.isRelayMediaUrl(e.url)) continue;
        m.getRelayMediaBlobUrl(e.url)
          .then((b) => !dead && setResolved((r) => (r[e.url] === b ? r : { ...r, [e.url]: b })))
          .catch(() => {});
      }
    });
    return () => {
      dead = true;
    };
  }, [emoji]);
  return React.useMemo(() => {
    if (!IS_WEB) return emoji;
    return emoji.flatMap((e) => (resolved[e.url] ? [{ ...e, url: resolved[e.url] }] : !looksLikeRelayMediaUrl(e.url) ? [e] : []));
  }, [emoji, resolved]);
}
