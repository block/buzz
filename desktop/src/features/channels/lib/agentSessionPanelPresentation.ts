/**
 * `AnimatePresence` key shared by every agent activity presentation.
 *
 * The split pane and the cover drawer are two containers for one session, so
 * presence is a property of the session, not of either container — crossing the
 * viewport breakpoint changes how it is shown, not whether it is open.
 */
export const AGENT_SESSION_SURFACE_KEY = "agent-session-surface";

export type AgentSessionPanelPresentation = {
  enterMotion: boolean;
  isSinglePanelView: boolean;
  layout: "standalone" | "split";
  transparentChrome: boolean;
};

type AgentSessionPanelPresentationOptions = {
  /** The panel is rendered inside the agent activity cover drawer. */
  isCoverDrawer: boolean;
  isSinglePanelView: boolean;
  useSplitAuxiliaryPane: boolean;
};

/**
 * Maps channel presentation into the agent session panel's layout props.
 *
 * TODO(#6538): once the `conversation` transcript variant lands on main, this
 * should also return `transcriptVariant: "conversation"` for the cover drawer
 * and `undefined` otherwise, so the reading view is pinned by presentation
 * rather than inferred from panel width. The variant does not exist on main
 * yet, so the prop is deliberately not set here.
 */
export function getAgentSessionPanelPresentation({
  isCoverDrawer,
  isSinglePanelView,
  useSplitAuxiliaryPane,
}: AgentSessionPanelPresentationOptions): AgentSessionPanelPresentation {
  if (isCoverDrawer) {
    return {
      // The drawer animates itself; a second slide inside it would compound.
      enterMotion: false,
      // Fills the drawer, and selects the standalone header chrome that owns
      // its own backdrop — the drawer is not sharing the channel's header, and
      // it has no resizable neighbour to draw a resize border against.
      isSinglePanelView: true,
      layout: "standalone",
      transparentChrome: false,
    };
  }

  return {
    enterMotion: true,
    isSinglePanelView: useSplitAuxiliaryPane ? false : isSinglePanelView,
    layout: useSplitAuxiliaryPane ? "split" : "standalone",
    transparentChrome: useSplitAuxiliaryPane,
  };
}
