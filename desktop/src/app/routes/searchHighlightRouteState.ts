import {
  parseSearchHighlightNavigation,
  type SearchHighlightNavigation,
} from "@/app/navigation/searchHighlightNavigation";

export function selectSearchHighlightRouteState(location: {
  state: unknown;
}): SearchHighlightNavigation | null | undefined {
  const state = location.state as { searchHighlight?: unknown } | undefined;
  if (!(state && "searchHighlight" in state)) {
    return undefined;
  }
  if (state.searchHighlight === null) {
    return null;
  }
  return parseSearchHighlightNavigation(state.searchHighlight) ?? undefined;
}
