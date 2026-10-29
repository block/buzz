import * as React from "react";

import { listenForEntityDeepLinks } from "@/shared/deep-link";
import { parseEntityLink } from "@/shared/lib/entityLink";
import { useOpenEntityLink } from "@/shared/ui/markdown/entityLinks";

/**
 * Subscribe to `buzz://repo|project|pr|issue` deep links emitted by the Tauri
 * backend and route them through the same handler that opens entity links
 * clicked inside a message, so an OS-opened share link and an in-app one land
 * on the same view.
 *
 * Mirrors `useMessageDeepLinks`: a hook rather than inline shell code so it
 * can be tested without the whole shell.
 */
export function useEntityDeepLinks(enabled = true) {
  const openEntityLink = useOpenEntityLink();

  React.useEffect(() => {
    if (!enabled) return;

    let cancelled = false;
    const unlistenPromise = listenForEntityDeepLinks((href) => {
      if (cancelled) return false;
      const parsed = parseEntityLink(href);
      // Ack-and-drop unparseable links: leaving one queued would wedge every
      // later link behind the bad head forever. Unmount (`cancelled`) keeps
      // links queued for the next listener instead.
      if (!parsed.ok) return true;
      openEntityLink(parsed.value);
      return true;
    });
    return () => {
      cancelled = true;
      void unlistenPromise.then((fn) => fn());
    };
  }, [enabled, openEntityLink]);
}
