import type { BusinessResource } from "@/features/business-dock/businessResourceResolver";

export type BusinessNavigationState = {
  entries: BusinessResource[];
  index: number;
};

export function createBusinessNavigationState(
  initial?: BusinessResource | null,
): BusinessNavigationState {
  return initial
    ? { entries: [initial], index: 0 }
    : { entries: [], index: -1 };
}

export function currentBusinessNavigationEntry(
  state: BusinessNavigationState,
): BusinessResource | null {
  return state.entries[state.index] ?? null;
}

export function pushBusinessNavigation(
  state: BusinessNavigationState,
  resource: BusinessResource,
): BusinessNavigationState {
  if (currentBusinessNavigationEntry(state)?.path === resource.path) {
    return updateCurrentBusinessNavigation(state, resource);
  }
  const entries = [...state.entries.slice(0, state.index + 1), resource];
  return { entries, index: entries.length - 1 };
}

export function updateCurrentBusinessNavigation(
  state: BusinessNavigationState,
  resource: BusinessResource,
): BusinessNavigationState {
  if (state.index < 0) return createBusinessNavigationState(resource);
  const entries = [...state.entries];
  entries[state.index] = resource;
  return { ...state, entries };
}

export function moveBusinessNavigation(
  state: BusinessNavigationState,
  direction: -1 | 1,
): BusinessNavigationState {
  const index = state.index + direction;
  return index >= 0 && index < state.entries.length
    ? { ...state, index }
    : state;
}

export function canMoveBusinessNavigation(
  state: BusinessNavigationState,
  direction: -1 | 1,
): boolean {
  const index = state.index + direction;
  return index >= 0 && index < state.entries.length;
}
