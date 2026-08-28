import * as React from "react";

/**
 * Lazy boundary for the LiveKit call view. Keeps `livekit-client` +
 * `@livekit/components-react` + their stylesheet in their own chunk so the SDK
 * never touches the main bundle — the view only loads when a user actually
 * joins a room (`?action=join`).
 */
export const LazyCallView = React.lazy(async () => {
  const module = await import("@/features/meetings/ui/CallView");
  return { default: module.CallView };
});
