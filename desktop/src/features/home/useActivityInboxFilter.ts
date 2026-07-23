import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import {
  type ActivityCustomView,
  type ActivityViewId,
  DEFAULT_ACTIVITY_VIEW_PREFERENCES,
  readActivityViewPreferences,
  writeActivityViewPreferences,
} from "@/features/home/lib/activityViewPreferences";
import type { InboxFilter } from "@/features/home/lib/inbox";

export function useActivityInboxFilter(
  activityEnabled: boolean,
  currentPubkey?: string,
) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl;
  const [preferences, setPreferences] = React.useState(
    DEFAULT_ACTIVITY_VIEW_PREFERENCES,
  );
  const [filter, setFilter] = React.useState<InboxFilter>(
    activityEnabled ? preferences.defaultView : "all",
  );
  const [unreadOnly, setUnreadOnly] = React.useState(false);
  React.useEffect(() => {
    const next = currentPubkey
      ? readActivityViewPreferences(currentPubkey, relayUrl)
      : DEFAULT_ACTIVITY_VIEW_PREFERENCES;
    setPreferences(next);
    setFilter(activityEnabled ? next.defaultView : "all");
  }, [activityEnabled, currentPubkey, relayUrl]);

  React.useEffect(() => {
    if (activityEnabled && filter === "activity") setFilter("all");
    if (!activityEnabled && filter === "custom") setFilter("all");
  }, [activityEnabled, filter]);

  const persist = React.useCallback(
    (next: typeof preferences) => {
      setPreferences(next);
      if (currentPubkey) {
        writeActivityViewPreferences(currentPubkey, next, relayUrl);
      }
    },
    [currentPubkey, relayUrl],
  );

  const setDefaultView = React.useCallback(
    (defaultView: ActivityViewId) => persist({ ...preferences, defaultView }),
    [persist, preferences],
  );

  const setCustomView = React.useCallback(
    (custom: ActivityCustomView) => persist({ ...preferences, custom }),
    [persist, preferences],
  );

  return {
    filter,
    preferences,
    setCustomView,
    setDefaultView,
    setFilter,
    setUnreadOnly,
    unreadOnly,
  };
}
