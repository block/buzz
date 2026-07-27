import * as React from "react";

import { clearDraftEntry } from "@/features/messages/lib/useDrafts";
import {
  useActiveDraftCount,
  useDraftViewItems,
} from "@/features/messages/ui/DraftsPanel";

type UseHomeDraftsOptions = {
  isDrafts: boolean;
  isNarrowHomeViewport: boolean;
  viewportWidthPx: number;
};

export function useHomeDrafts({
  isDrafts,
  isNarrowHomeViewport,
  viewportWidthPx,
}: UseHomeDraftsOptions) {
  const items = useDraftViewItems(isDrafts);
  const optimisticActiveCount = useActiveDraftCount(new Map());
  const activeCount = isDrafts
    ? items.filter((item) => item.rootStatus !== "deleted").length
    : optimisticActiveCount;
  const [selectedKey, setSelectedKey] = React.useState<string | null>(null);
  const selectedItem =
    items.find((item) => item.entry.key === selectedKey) ?? null;

  React.useEffect(() => {
    if (!isDrafts) {
      setSelectedKey(null);
      return;
    }
    if (viewportWidthPx === 0) {
      return;
    }
    if (
      selectedKey !== null &&
      items.some((item) => item.entry.key === selectedKey)
    ) {
      return;
    }
    setSelectedKey(isNarrowHomeViewport ? null : (items[0]?.entry.key ?? null));
  }, [isDrafts, isNarrowHomeViewport, items, selectedKey, viewportWidthPx]);

  const deleteDraft = React.useCallback(
    (draftKey: string) => {
      clearDraftEntry(draftKey);
      if (selectedKey === draftKey) {
        setSelectedKey(null);
      }
    },
    [selectedKey],
  );

  return {
    activeCount,
    deleteDraft,
    items,
    selectedItem,
    selectedKey,
    selectDraft: setSelectedKey,
  };
}
