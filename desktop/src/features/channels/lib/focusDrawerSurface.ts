/**
 * Which auxiliary surface owns the focus-mode drawer.
 *
 * Focus mode is a *presentation* of the right-hand auxiliary region, not a
 * thread feature: the channel agent work viewer honours the same
 * `threadViewMode` preference and reuses the same drawer. Only one surface can
 * hold the drawer at a time, so the winner has to be resolved from the same
 * precedence the render chain uses — otherwise the drawer and the split pane
 * disagree about who is on screen.
 */

/** `AnimatePresence` key shared by both agent-session layouts.
 *
 * The split pane and the focus drawer are two containers for one viewer, so
 * presence belongs to the viewer rather than either container — keying them
 * apart would make every layout switch read as a close followed by an open.
 * Mirrors `THREAD_SURFACE_KEY`.
 */
export const AGENT_SESSION_SURFACE_KEY = "agent-session-surface";

export type FocusDrawerSurface = "agent-session" | "thread";

type FocusDrawerSurfaceOptions = {
  /**
   * Channel management is open *and* has a channel to manage — pass the same
   * conjunction the render chain branches on, since it outranks both drawers.
   */
  channelManagementOpen: boolean;
  /** An agent work viewer has a resolved agent and channel to render. */
  hasAgentSession: boolean;
  /** A thread panel or its skeleton is on screen. */
  hasThread: boolean;
  /** The persisted `threadViewMode` preference selects the drawer. */
  isFocusPreferred: boolean;
  /** False in the narrow single-column and overlay presentations. */
  useSplitAuxiliaryPane: boolean;
};

/**
 * Resolves the focus drawer's occupant, or `null` when nothing should overlay.
 *
 * Precedence deliberately mirrors `ChannelPane`'s auxiliary render chain
 * (management → thread → agent session). Keeping one ordering means a surface
 * cannot be routed into the drawer while a higher-precedence surface is what
 * actually renders.
 */
export function resolveFocusDrawerSurface({
  channelManagementOpen,
  hasAgentSession,
  hasThread,
  isFocusPreferred,
  useSplitAuxiliaryPane,
}: FocusDrawerSurfaceOptions): FocusDrawerSurface | null {
  // The narrow and overlay presentations are unchanged by focus mode: they
  // already fill the column, so there is no channel left to dim behind a scrim.
  if (!isFocusPreferred || !useSplitAuxiliaryPane) return null;
  // Channel management keeps its split pane in every view mode.
  if (channelManagementOpen) return null;
  if (hasThread) return "thread";
  if (hasAgentSession) return "agent-session";
  return null;
}
