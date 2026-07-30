import * as React from "react";

import { useMcpAppUi } from "@/features/mcp-apps/lib/useChannelMcpAppExperience";
import type { Channel } from "@/shared/api/types";

/**
 * Publishes a message to a channel on the viewer's behalf.
 *
 * Surfaces receive this so they can request a post without reaching into the
 * channel shell's mutation wiring.
 */
export type SendChannelMessage = (
  content: string,
  mentionPubkeys: string[],
  mediaTags?: string[][],
  channelId?: string | null,
) => Promise<void>;

/** Everything a surface needs to resolve itself for the active channel. */
export type ChannelSurfaceContext = {
  channel: Channel | null;
  pubkey: string | null | undefined;
  /** Stable Buzz community reference, when the shell has one. */
  communityRef?: string | null;
  sendMessage: SendChannelMessage;
};

/**
 * What a channel surface contributes to the channel shell.
 *
 * Chrome and content are independent: a surface may contribute header chrome
 * (such as a tab strip that lets the viewer select it) while the channel still
 * shows its default content. It takes over the content region only when it
 * supplies `renderContent`.
 */
export type ChannelSurfacePresentation = {
  /** Chrome rendered in the channel header, such as a tab strip. */
  navigation?: React.ReactNode;
  /**
   * Renders the surface in place of the default channel content. Receives the
   * channel header so a full-bleed surface can position its own chrome. When
   * absent, the channel keeps its default content.
   */
  renderContent?: (header: React.ReactNode) => React.ReactNode;
};

/** The subset of a surface feature's state that the shell maps into a presentation. */
export type ChannelSurfaceSource = {
  active: boolean;
  navigation?: React.ReactNode;
  renderPane: (header: React.ReactNode) => React.ReactNode;
};

/**
 * Maps a surface feature's state into a shell presentation.
 *
 * Returns a presentation whenever the feature has anything to contribute, so a
 * feature that only offers header chrome still renders it. Gating the whole
 * presentation on `active` would hide the very chrome used to activate it.
 */
export function resolveChannelSurface(
  source: ChannelSurfaceSource,
): ChannelSurfacePresentation | null {
  if (!source.active && source.navigation === undefined) {
    return null;
  }
  return {
    navigation: source.navigation,
    renderContent: source.active ? source.renderPane : undefined,
  };
}

/**
 * Resolves the channel surface for the active channel, or `null` when no
 * surface contributes anything.
 *
 * Surfaces are composed explicitly rather than iterated so that every hook call
 * stays unconditional and in a fixed order. Adding a surface means adding one
 * resolver here; the channel shell does not change.
 */
export function useChannelSurface(
  context: ChannelSurfaceContext,
): ChannelSurfacePresentation | null {
  const { active, navigation, renderPane } = useMcpAppUi(
    context.channel,
    context.pubkey,
    context.sendMessage,
    context.communityRef,
  );

  return React.useMemo(
    () => resolveChannelSurface({ active, navigation, renderPane }),
    [active, navigation, renderPane],
  );
}
