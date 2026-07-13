import * as React from "react";

import type { MarkdownRuntime } from "./types";

/**
 * Per-mount runtime (channels, imeta lookup, navigation callbacks) flows
 * through context for the same reason as `VideoReviewMarkdownContext` in
 * markdown.tsx: the component map must stay identity-stable. Routing runtime
 * through context (read at render time) rather than a closed-over ref
 * additionally makes the map module-stable across mounts, which is what
 * allows the parsed markdown element cache (`nodeCache.ts`) to reuse element
 * trees across the timeline's per-channel-switch remount without capturing
 * stale per-mount state.
 */
const INERT_MARKDOWN_RUNTIME: MarkdownRuntime = {
  channels: [],
  onOpenChannel: () => {},
  onOpenMessageLink: () => {},
};

export const MarkdownRuntimeContext = React.createContext<MarkdownRuntime>(
  INERT_MARKDOWN_RUNTIME,
);

export function useMarkdownRuntime(): MarkdownRuntime {
  return React.useContext(MarkdownRuntimeContext);
}
