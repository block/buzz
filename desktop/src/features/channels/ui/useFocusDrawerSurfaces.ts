import {
  resolveFocusDrawerSurface,
  type FocusDrawerSurface,
} from "@/features/channels/lib/focusDrawerSurface";
import { AGENT_SESSION_BODY_TEST_ID } from "@/features/channels/ui/AgentSessionThreadPanel";
import { useFocusDrawerPresence } from "@/features/channels/ui/useFocusDrawerPresence";
import { useThreadViewModeSwitch } from "@/features/channels/ui/useThreadViewModeSwitch";

const AGENT_SESSION_BODY_SELECTOR = `[data-testid="${AGENT_SESSION_BODY_TEST_ID}"]`;

type UseFocusDrawerSurfacesOptions = {
  channelManagementOpen: boolean;
  hasAgentSession: boolean;
  hasThread: boolean;
  isFocusPreferred: boolean;
  onCloseAgentSession: () => void;
  onCloseThread: () => void;
  onThreadScrollTargetResolved: () => void;
  threadHeadId: string | null;
  threadScrollTargetId: string | null;
  useSplitAuxiliaryPane: boolean;
};

type ThreadViewModeSwitchHandler = ReturnType<
  typeof useThreadViewModeSwitch
>["changeThreadViewMode"];

type UseFocusDrawerSurfacesResult = {
  /** Keeps the covered channel section inert while the drawer is on screen. */
  channelIsCovered: boolean;
  changeAgentViewMode: ThreadViewModeSwitchHandler;
  changeThreadViewMode: ThreadViewModeSwitchHandler;
  focusDrawerSurface: FocusDrawerSurface | null;
  layoutScrollTargetId: string | null;
  /** Hand to the auxiliary `AnimatePresence` so the drawer can finish exiting. */
  markExitComplete: () => void;
  resolveScrollTarget: (settledMessageId?: string) => void;
};

/**
 * Resolves which auxiliary surface occupies the focus drawer, and wires the
 * per-surface presentation state that follows from it.
 *
 * Threads and the agent work viewer are two occupants of one drawer, so the
 * drawer's presence, dismissal ownership and layout toggles all have to be
 * decided together from a single resolution — deciding them separately is how
 * the drawer and the split pane end up disagreeing about who is on screen.
 */
export function useFocusDrawerSurfaces({
  channelManagementOpen,
  hasAgentSession,
  hasThread,
  isFocusPreferred,
  onCloseAgentSession,
  onCloseThread,
  onThreadScrollTargetResolved,
  threadHeadId,
  threadScrollTargetId,
  useSplitAuxiliaryPane,
}: UseFocusDrawerSurfacesOptions): UseFocusDrawerSurfacesResult {
  const focusDrawerSurface = resolveFocusDrawerSurface({
    channelManagementOpen,
    hasAgentSession,
    hasThread,
    isFocusPreferred,
    useSplitAuxiliaryPane,
  });
  // Whichever surface holds the drawer owns its dismissal, so presence tracks
  // the drawer as one overlay and closes the surface actually on screen.
  const closeFocusDrawerSurface =
    focusDrawerSurface === "agent-session"
      ? onCloseAgentSession
      : onCloseThread;
  const { channelIsCovered, markExitComplete } = useFocusDrawerPresence(
    focusDrawerSurface !== null,
    closeFocusDrawerSurface,
  );
  const { changeThreadViewMode, layoutScrollTargetId, resolveScrollTarget } =
    useThreadViewModeSwitch({
      activeThreadHeadId: threadHeadId,
      externalScrollTargetId: threadScrollTargetId,
      onExternalTargetResolved: onThreadScrollTargetResolved,
      onModeChange: markExitComplete,
    });
  // The viewer gets its own switch instance so the toggle it renders writes the
  // shared preference without borrowing the thread's captured anchor. It
  // deliberately does not preserve reading position — see
  // `preserveReadingPosition`.
  const { changeThreadViewMode: changeAgentViewMode } = useThreadViewModeSwitch(
    {
      activeThreadHeadId: null,
      bodySelector: AGENT_SESSION_BODY_SELECTOR,
      onModeChange: markExitComplete,
      preserveReadingPosition: false,
    },
  );

  return {
    channelIsCovered,
    changeAgentViewMode,
    changeThreadViewMode,
    focusDrawerSurface,
    layoutScrollTargetId,
    markExitComplete,
    resolveScrollTarget,
  };
}
