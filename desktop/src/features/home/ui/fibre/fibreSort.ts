import { FIBRE_LANES, type Fibre, type FibreLane } from "@/features/triage/api";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

export type FibreSort = "priority" | "newest";
export type FibreListTab = FibreLane | "done";

export const FIBRE_LIST_TABS: FibreListTab[] = [...FIBRE_LANES, "done"];

export const FIBRE_TAB_LABELS: Record<FibreListTab, string> = {
  important: "Important",
  hot: "Hot",
  other: "Other",
  done: "Done",
};

export function isFibreListTab(
  value: string | null | undefined,
): value is FibreListTab {
  return FIBRE_LIST_TABS.includes(value as FibreListTab);
}

export const FIBRE_SORT_STORAGE_KEY = "buzz-fibre-sort.v1";

export function isFibreSort(
  value: string | null | undefined,
): value is FibreSort {
  return value === "priority" || value === "newest";
}

export function readFibreSort(): FibreSort | null {
  if (typeof window === "undefined") return null;
  try {
    const stored = window.localStorage.getItem(FIBRE_SORT_STORAGE_KEY);
    return isFibreSort(stored) ? stored : null;
  } catch {
    return null;
  }
}

export function writeFibreSort(sort: FibreSort): void {
  if (typeof window === "undefined") return;
  setLocalStorageItemWithRecovery(FIBRE_SORT_STORAGE_KEY, sort);
}

/**
 * Important is ranked, so priority reads best there. Hot, Other, and Done are
 * about what is happening rather than what matters most, so they default to
 * newest until the user picks.
 */
export function resolveFibreSort(
  tab: FibreListTab,
  preference: FibreSort | null,
): FibreSort {
  return preference ?? (tab === "important" ? "priority" : "newest");
}

export function fibreActivityAt(fibre: Fibre): number {
  return (
    fibre.artifacts.reduce(
      (latest, artifact) => Math.max(latest, artifact.createdAt ?? 0),
      fibre.updatedAt,
    ) || fibre.updatedAt
  );
}

export function sortFibres(
  fibres: readonly Fibre[],
  sort: FibreSort,
  recency: "activity" | "updated" = "activity",
): Fibre[] {
  return [...fibres].sort((left, right) => {
    if (sort === "newest") {
      const leftAt =
        recency === "updated" ? (left.updatedAt ?? 0) : fibreActivityAt(left);
      const rightAt =
        recency === "updated" ? (right.updatedAt ?? 0) : fibreActivityAt(right);
      if (rightAt !== leftAt) return rightAt - leftAt;
      return right.score - left.score;
    }
    if (right.score !== left.score) return right.score - left.score;
    return (right.updatedAt ?? 0) - (left.updatedAt ?? 0);
  });
}
