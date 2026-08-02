import * as React from "react";

import { clearDraftEntry } from "@/features/messages/lib/useDrafts";
import {
  useActiveDraftCount,
  useDraftViewItems,
} from "@/features/messages/ui/DraftsPanel";

type UseHomeDraftsOptions = {
  autoSelect: boolean;
  isNarrowHomeViewport: boolean;
  selectionEnabled: boolean;
  viewportWidthPx: number;
};

export function useHomeDrafts({
  autoSelect,
  isNarrowHomeViewport,
  selectionEnabled,
  viewportWidthPx,
}: UseHomeDraftsOptions) {
  const items = useDraftViewItems(selectionEnabled);
  const optimisticActiveCount = useActiveDraftCount(new Map());
  const activeCount = selectionEnabled
    ? items.filter((item) => item.rootStatus !== "deleted").length
    : optimisticActiveCount;
  const [selectedKey, setSelectedKey] = React.useState<string | null>(null);
  const selectedItem =
    items.find((item) => item.entry.key === selectedKey) ?? null;

  React.useEffect(() => {
    if (!selectionEnabled) {
      setSelectedKey(null);
      return;
    }
    if (
      selectedKey !== null &&
      !items.some((item) => item.entry.key === selectedKey)
    ) {
      setSelectedKey(null);
      return;
    }
    if (!autoSelect) return;
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
  }, [
    autoSelect,
    isNarrowHomeViewport,
    items,
    selectedKey,
    selectionEnabled,
    viewportWidthPx,
  ]);

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
