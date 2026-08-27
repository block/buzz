import type { AgentSessionTranscriptVariant } from "@/features/agents/ui/agentSessionTranscriptContext";

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
  transcriptVariant: AgentSessionTranscriptVariant | undefined;
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
 * The transcript variant is pinned here rather than inferred from panel width.
 * The cover drawer is the reading surface, so it gets `conversation`; every
 * other host keeps the dense activity feed. Width is a proxy that breaks — the
 * split pane can be dragged wide and a narrow overlay can be tall — so the
 * presentation that decided to cover is what decides the reading view too.
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
      transcriptVariant: "conversation",
      transparentChrome: false,
    };
  }

  return {
    enterMotion: true,
    isSinglePanelView: useSplitAuxiliaryPane ? false : isSinglePanelView,
    layout: useSplitAuxiliaryPane ? "split" : "standalone",
    // Undefined, not `"default"`: the panel already defaults, and naming it
    // here would claim this function decides the non-cover variant when the
    // profile panel's `compactPreview` is chosen at its own call site.
    transcriptVariant: undefined,
    transparentChrome: useSplitAuxiliaryPane,
  };
}
