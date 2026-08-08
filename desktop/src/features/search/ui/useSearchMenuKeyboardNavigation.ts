import * as React from "react";

import type { SearchResult } from "@/features/search/ui/SearchResultItem";

export function useSearchMenuKeyboardNavigation({
  activeResults,
  onOpenResult,
  onRemoveScope,
  query,
  scopeActive,
  selectedMenuIndex,
  setSelectedMenuIndex,
}: {
  activeResults: SearchResult[];
  onOpenResult: (result: SearchResult) => void;
  onRemoveScope: () => void;
  query: string;
  scopeActive: boolean;
  selectedMenuIndex: number;
  setSelectedMenuIndex: React.Dispatch<React.SetStateAction<number>>;
}) {
  React.useEffect(() => {
    setSelectedMenuIndex((current) => {
      if (activeResults.length === 0) return 0;
      return Math.min(current, activeResults.length - 1);
    });
  }, [activeResults.length, setSelectedMenuIndex]);

  React.useEffect(() => {
    const selectedResult = document.querySelector<HTMLElement>(
      `[data-search-result-index="${selectedMenuIndex}"]`,
    );
    selectedResult?.scrollIntoView({ block: "nearest" });
  }, [selectedMenuIndex]);

  const handleDialogInputKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "Backspace" && query.length === 0 && scopeActive) {
        event.preventDefault();
        onRemoveScope();
        return;
      }

      if (event.key === "ArrowDown" && activeResults.length > 0) {
        event.preventDefault();
        setSelectedMenuIndex((current) =>
          Math.min(current + 1, activeResults.length - 1),
        );
        return;
      }

      if (event.key === "ArrowUp" && activeResults.length > 0) {
        event.preventDefault();
        setSelectedMenuIndex((current) => Math.max(current - 1, 0));
        return;
      }

      if (event.key === "Enter" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        const result = activeResults[selectedMenuIndex];
        if (result) onOpenResult(result);
      }
    },
    [
      activeResults,
      onOpenResult,
      onRemoveScope,
      query.length,
      scopeActive,
      selectedMenuIndex,
      setSelectedMenuIndex,
    ],
  );

  return handleDialogInputKeyDown;
}
