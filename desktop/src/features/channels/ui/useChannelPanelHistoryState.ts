import * as React from "react";

import type { ProfilePanelView } from "@/features/profile/ui/UserProfilePanel";
import {
  type HistorySearchSetterOptions,
  useHistorySearchState,
} from "@/shared/hooks/useHistorySearchState";

/**
 * Auxiliary-panel state for the channel routes, backed by URL search params
 * via useHistorySearchState: back/forward restores the panel a given entry
 * was showing, and reloads restore the panel from the URL.
 *
 * Params: `thread` (open thread head id), `profile` (profile panel pubkey),
 * `profileView` (profile panel sub-view), `agentSession` (agent session
 * panel pubkey).
 */

export type PanelSetterOptions = HistorySearchSetterOptions;

export type PanelValueSetter = (
  value: string | null,
  options?: PanelSetterOptions,
) => void;

const CHANNEL_SEARCH_KEYS = [
  "agentSession",
  "messageId",
  "profile",
  "profileView",
  "thread",
  "threadRootId",
] as const;

function asProfilePanelView(value: string | null): ProfilePanelView {
  return value === "memories" || value === "channels" ? value : "summary";
}

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
      applyPatch({ profile: value, profileView: null }, options),
    [applyPatch],
  );

  const setProfilePanelView = React.useCallback(
    (value: ProfilePanelView, options?: PanelSetterOptions) =>
      applyPatch({ profileView: value === "summary" ? null : value }, options),
    [applyPatch],
  );

  const setOpenAgentSessionPubkey = React.useCallback<PanelValueSetter>(
    (value, options) => applyPatch({ agentSession: value }, options),
    [applyPatch],
  );

  const clearMessageRouteTarget = React.useCallback(
    (options?: PanelSetterOptions) =>
      applyPatch({ messageId: null, threadRootId: null }, options),
    [applyPatch],
  );

  return {
    clearMessageRouteTarget,
    openAgentSessionPubkey: values.agentSession,
    openThreadHeadId: values.thread,
    profilePanelPubkey: values.profile,
    profilePanelView: asProfilePanelView(values.profileView),
    setOpenAgentSessionPubkey,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setProfilePanelView,
  };
}
