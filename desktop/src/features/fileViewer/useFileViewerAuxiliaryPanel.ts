import * as React from "react";

import { closeFileViewer, selectActiveFileViewerTab } from "./fileViewerStore";
import { useFileViewerState } from "./useFileViewerState";

type AuxiliaryPanelKeys = {
  agentSession: string | null | undefined;
  channelManagement: boolean;
  profile: string | null | undefined;
  thread: string | null | undefined;
};

/**
 * Whether the file viewer currently claims the channel's auxiliary-pane slot.
 *
 * Also closes the viewer when another auxiliary panel opens or retargets: the
 * slot holds one panel and the viewer branch renders first, so a newly opened
 * thread/profile/activity panel would otherwise stay hidden behind it. Only
 * transitions close the viewer — panels already open on mount (restored from
 * the URL) leave it alone.
 */
export function useFileViewerAuxiliaryPanel(
  panelKeys: AuxiliaryPanelKeys,
): boolean {
  const snapshot = useFileViewerState();
  const previousKeysRef = React.useRef(panelKeys);
  const { agentSession, channelManagement, profile, thread } = panelKeys;

  React.useEffect(() => {
    const previous = previousKeysRef.current;
    const next = { agentSession, channelManagement, profile, thread };
    previousKeysRef.current = next;
    const otherPanelOpened =
      (next.thread && next.thread !== previous.thread) ||
      (next.agentSession && next.agentSession !== previous.agentSession) ||
      (next.profile && next.profile !== previous.profile) ||
      (next.channelManagement && !previous.channelManagement);
    if (otherPanelOpened) closeFileViewer();
  }, [agentSession, channelManagement, profile, thread]);

  return selectActiveFileViewerTab(snapshot) !== null;
}
