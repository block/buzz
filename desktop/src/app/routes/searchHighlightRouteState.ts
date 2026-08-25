import {
  parseSearchHighlightNavigation,
  type SearchHighlightNavigation,
} from "@/app/navigation/searchHighlightNavigation";

export function selectSearchHighlightRouteState(location: {
  state: unknown;
}): SearchHighlightNavigation | null {
  return parseSearchHighlightNavigation(
    (location.state as { searchHighlight?: unknown } | undefined)
      ?.searchHighlight,
  );
}
