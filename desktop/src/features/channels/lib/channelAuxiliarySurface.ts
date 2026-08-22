import type { ThreadViewMode } from "@/features/channels/lib/threadViewModePreference";

/**
 * The one auxiliary surface a channel shows beside (or over) its timeline.
 *
 * Exactly one at a time, in the fixed priority order below.
 *
 * This priority is a **safety net, not the product rule.** Last-opened-wins is
 * implemented by the open handlers, which clear the competing state as they open
 * (`useChannelAgentSessions`: `openAgentSession` clears the thread head, and
 * `openThreadAndCloseAgentSession` clears the agent session). By the time this
 * resolver runs, at most one candidate should normally be live.
 *
 * The ordering only decides cases the handlers cannot: two candidates present at
 * once with no ordering information between them — a restored/hand-edited URL
 * carrying both `agentSession` and a thread param, or a stale param that has not
 * been reconciled yet. Then it picks deterministically instead of rendering two
 * surfaces. Do not read priority as "thread beats agent" in the UX; a thread
 * opened while activity is up wins because the handler cleared the agent
 * session, and activity opened over a thread wins for the same reason.
 */
export type ChannelAuxiliarySurface =
  | "agent-session"
  | "channel-management"
  | "profile"
  | "thread"
  | "thread-skeleton";

type ChannelAuxiliarySurfaceOptions = {
  channelManagementOpen: boolean;
  hasActiveChannel: boolean;
  hasProfilePanel: boolean;
  hasSelectedAgent: boolean;
  hasThreadHead: boolean;
  shouldShowThreadSkeleton: boolean;
};

/** Which auxiliary surface the channel pane should render, if any. */
export function resolveChannelAuxiliarySurface({
  channelManagementOpen,
  hasActiveChannel,
  hasProfilePanel,
  hasSelectedAgent,
  hasThreadHead,
  shouldShowThreadSkeleton,
}: ChannelAuxiliarySurfaceOptions): ChannelAuxiliarySurface | null {
  if (channelManagementOpen && hasActiveChannel) return "channel-management";
  if (hasThreadHead) return "thread";
  if (shouldShowThreadSkeleton) return "thread-skeleton";
  if (hasActiveChannel && hasSelectedAgent) return "agent-session";
  if (hasProfilePanel) return "profile";
  return null;
}

/** A cover drawer overlays the channel content area instead of splitting it. */
export type ChannelCoverDrawer = "agent-session" | "thread";

type ChannelCoverDrawerOptions = {
  surface: ChannelAuxiliarySurface | null;
  threadViewMode: ThreadViewMode;
  useSplitAuxiliaryPane: boolean;
};

/**
 * Which surface, if any, presents as a cover drawer.
 *
 * Threads honour the user's view-mode preference. Agent activity does not and
 * deliberately offers no toggle: its transcript is tool calls, diffs, and
 * command output, which a 380px side pane cannot show usefully — so at any
 * viewport wide enough for two panes it always covers. Narrow/overlay and
 * single-panel viewports keep their existing presentations for both.
 *
 * Returning a single value is what makes the two drawers mutually exclusive:
 * there is one covered slot, and the resolved surface owns it.
 */
export function resolveChannelCoverDrawer({
  surface,
  threadViewMode,
  useSplitAuxiliaryPane,
}: ChannelCoverDrawerOptions): ChannelCoverDrawer | null {
  if (!useSplitAuxiliaryPane) return null;

  if (surface === "thread" || surface === "thread-skeleton") {
    return threadViewMode === "focus" ? "thread" : null;
  }

  return surface === "agent-session" ? "agent-session" : null;
}
