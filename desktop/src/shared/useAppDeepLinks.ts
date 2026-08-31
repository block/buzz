import { useEntityDeepLinks } from "@/shared/useEntityDeepLinks";
import { useMessageDeepLinks } from "@/shared/useMessageDeepLinks";

/**
 * Subscribe to every deep link that routes inside the app shell —
 * `buzz://message` plus the `buzz://repo|project|pr|issue` share links.
 *
 * Both need the router, so they mount together in `AppShell`; bundling them
 * here keeps the shell to one line per concern.
 */
export function useAppDeepLinks(enabled = true) {
  useMessageDeepLinks(enabled);
  useEntityDeepLinks(enabled);
}
