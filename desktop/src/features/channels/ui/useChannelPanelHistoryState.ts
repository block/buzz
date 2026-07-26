import * as React from "react";

import {
  profilePanelTabFromSearch,
  type ProfilePanelTab,
  profilePanelViewFromSearch,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  type HistorySearchSetterOptions,
  useHistorySearchState,
} from "@/shared/hooks/useHistorySearchState";
import {
  buildAutoSendClearPatch,
  CHANNEL_SEARCH_KEYS,
} from "./channelSearchKeys";
export type { ChannelSearchKey } from "./channelSearchKeys";

/**
 * Auxiliary-panel state for the channel routes, backed by URL search params
 * via useHistorySearchState: back/forward restores the panel a given entry
 * was showing, and reloads restore the panel from the URL.
 *
 * Params: `thread` (open thread head id), `profile` (profile panel pubkey),
 * `profileView` (profile panel focused view), `profileTab` (profile summary
 * tab), `agentSession` (agent session panel pubkey), `agentSessionChannel`
 * (optional channel scope for the agent session panel), `channelManagement`
 * (presence flag for the channel-management panel — open/closed only, so it
 * carries a sentinel `"1"` rather than an id), `harness` (presence flag
 * promoting the agent session to the full-screen harness view — also a
 * sentinel `"1"`, and only meaningful alongside `agentSession`), `autoSend`
 * (draft auto-submit trigger — cleared surgically after the auto-submit fires
 * so `thread` and all other panel state are preserved).
 */

export type PanelSetterOptions = HistorySearchSetterOptions;

export type PanelValueSetter = (
  value: string | null,
  options?: PanelSetterOptions,
) => void;

const CHANNEL_MANAGEMENT_OPEN_VALUE = "1";
const HARNESS_OPEN_VALUE = "1";

export function useChannelPanelHistoryState() {
  const { applyPatch, values } = useHistorySearchState(CHANNEL_SEARCH_KEYS);

  const setOpenThreadHeadId = React.useCallback<PanelValueSetter>(
    (value, options) => applyPatch({ thread: value }, options),
    [applyPatch],
  );

  // Opening, switching, or closing a profile always resets its sub-view —
  // the carried `profileView` would otherwise leak onto the next profile.
  const setProfilePanelPubkey = React.useCallback<PanelValueSetter>(
    (value, options) =>
      applyPatch(
        { profile: value, profileTab: null, profileView: null },
        options,
      ),
    [applyPatch],
  );

  const setProfilePanelView = React.useCallback(
    (value: ProfilePanelView, options?: PanelSetterOptions) =>
      applyPatch({ profileView: value === "summary" ? null : value }, options),
    [applyPatch],
  );

  const setProfilePanelTab = React.useCallback(
    (value: ProfilePanelTab, options?: PanelSetterOptions) =>
      applyPatch({ profileTab: value === "info" ? null : value }, options),
    [applyPatch],
  );

  // Closing the agent session also leaves harness mode — a `harness` flag with
  // no `agentSession` has nothing to render, so it must never outlive it.
  const setOpenAgentSessionPubkey = React.useCallback<PanelValueSetter>(
    (value, options) =>
      applyPatch(
        {
          agentSession: value,
          agentSessionChannel: value ? undefined : null,
          harness: value ? undefined : null,
        },
        options,
      ),
    [applyPatch],
  );

  const setHarnessOpen = React.useCallback(
    (open: boolean, options?: PanelSetterOptions) =>
      applyPatch({ harness: open ? HARNESS_OPEN_VALUE : null }, options),
    [applyPatch],
  );

  // Selecting the agent and raising the harness flag must land in ONE patch:
  // two sequential applyPatch calls in the same tick both derive from the
  // pre-patch search state, so the second would drop the first's key.
  const openHarnessForAgent = React.useCallback(
    (
      agentPubkey: string,
      channelId?: string | null,
      options?: PanelSetterOptions,
    ) =>
      applyPatch(
        {
          agentSession: agentPubkey,
          agentSessionChannel: channelId ?? undefined,
          harness: HARNESS_OPEN_VALUE,
        },
        options,
      ),
    [applyPatch],
  );

  const setOpenAgentSessionChannelId = React.useCallback<PanelValueSetter>(
    (value, options) => applyPatch({ agentSessionChannel: value }, options),
    [applyPatch],
  );

  const setChannelManagementOpen = React.useCallback(
    (open: boolean, options?: PanelSetterOptions) =>
      applyPatch(
        { channelManagement: open ? CHANNEL_MANAGEMENT_OPEN_VALUE : null },
        options,
      ),
    [applyPatch],
  );

  const clearMessageRouteTarget = React.useCallback(
    (options?: PanelSetterOptions) =>
      applyPatch({ messageId: null, threadRootId: null }, options),
    [applyPatch],
  );

  // Clears only the ?autoSend param, preserving `thread` and all other panel
  // search state. Use this instead of a full goChannel() re-navigation so the
  // thread panel does not unmount between the auto-submit trigger clear and the
  // deferred setTimeout(0) send.
  const clearAutoSend = React.useCallback(
    (options?: PanelSetterOptions) =>
      applyPatch(buildAutoSendClearPatch(), { replace: true, ...options }),
    [applyPatch],
  );

  return {
    channelManagementOpen: values.channelManagement != null,
    clearAutoSend,
    clearMessageRouteTarget,
    harnessOpen: values.harness != null,
    openHarnessForAgent,
    openAgentSessionChannelId: values.agentSessionChannel,
    openAgentSessionPubkey: values.agentSession,
    openThreadHeadId: values.thread,
    profilePanelPubkey: values.profile,
    profilePanelTab: profilePanelTabFromSearch(values.profileTab),
    profilePanelView: profilePanelViewFromSearch(values.profileView),
    setChannelManagementOpen,
    setHarnessOpen,
    setOpenAgentSessionChannelId,
    setOpenAgentSessionPubkey,
    setOpenThreadHeadId,
    setProfilePanelTab,
    setProfilePanelPubkey,
    setProfilePanelView,
  };
}
