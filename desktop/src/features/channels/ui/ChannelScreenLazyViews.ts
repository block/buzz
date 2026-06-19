import * as React from "react";

// Hoisted chunk imports so each view can be warmed eagerly (route preload) as
// well as loaded lazily on render; the module loader dedupes repeat calls.
const importChannelPane = () => import("@/features/channels/ui/ChannelPane");
const importForumView = () => import("@/features/forum/ui/ForumView");

export const ChannelPane = React.lazy(async () => {
  const module = await importChannelPane();
  return { default: module.ChannelPane };
});

export const ForumView = React.lazy(async () => {
  const module = await importForumView();
  return { default: module.ForumView };
});

/** Warms the channel/forum view chunks so first open doesn't stall. */
export function preloadChannelViews(): void {
  void importChannelPane();
  void importForumView();
}
