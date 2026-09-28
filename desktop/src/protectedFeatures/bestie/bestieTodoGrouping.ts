import {
  localDayKeyFromSeconds,
  todayLocalDayKey,
} from "./bestieListStorage";
import type { BestieListItem } from "./bestieListTypes";

export type BestieTodoDayGroup = {
  dayKey: string;
  items: BestieListItem[];
  label: string;
};

function compareSortOrder(a: BestieListItem, b: BestieListItem): number {
  if (a.sortOrder !== b.sortOrder) return a.sortOrder - b.sortOrder;
  return a.createdAt - b.createdAt;
}

function dayLabel(dayKey: string, todayKey: string): string {
  if (dayKey === todayKey) return "Today";
  const today = new Date(`${todayKey}T12:00:00`);
  const day = new Date(`${dayKey}T12:00:00`);
  const diffDays = Math.round(
    (today.getTime() - day.getTime()) / (24 * 60 * 60 * 1000),
  );
  if (diffDays === 1) return "Yesterday";
  if (diffDays === -1) return "Tomorrow";
  return day.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

/**
 * Split open todos into starred (always on top) + day groups (Today first).
 * Done todos are omitted from the grouped view (callers may list separately).
 */
export function groupBestieTodos(
  items: readonly BestieListItem[],
  nowSeconds = Math.floor(Date.now() / 1000),
): {
  dayGroups: BestieTodoDayGroup[];
  starred: BestieListItem[];
} {
  const todayKey = todayLocalDayKey(nowSeconds);
  const openTodos = items.filter(
    (item) => item.kind === "todo" && item.status === "open",
  );
  const starred = openTodos
    .filter((item) => item.starred)
    .slice()
    .sort(compareSortOrder);

  const byDay = new Map<string, BestieListItem[]>();
  for (const item of openTodos) {
    if (item.starred) continue;
    const key =
      item.dayKey && /^\d{4}-\d{2}-\d{2}$/.test(item.dayKey)
        ? item.dayKey
        : localDayKeyFromSeconds(item.createdAt);
    const list = byDay.get(key) ?? [];
    list.push(item);
    byDay.set(key, list);
  }

  const keys = [...byDay.keys()].sort((a, b) => {
    if (a === todayKey) return -1;
    if (b === todayKey) return 1;
    // Future days after Today, then past descending
    if (a > todayKey && b > todayKey) return a.localeCompare(b);
    if (a > todayKey) return -1;
    if (b > todayKey) return 1;
    return b.localeCompare(a);
  });

  const dayGroups: BestieTodoDayGroup[] = keys.map((dayKey) => ({
    dayKey,
    items: (byDay.get(dayKey) ?? []).slice().sort(compareSortOrder),
    label: dayLabel(dayKey, todayKey),
  }));

  // Ensure Today group exists so users can drag into it even when empty.
  if (!dayGroups.some((group) => group.dayKey === todayKey)) {
    dayGroups.unshift({ dayKey: todayKey, items: [], label: "Today" });
  }

  return { dayGroups, starred };
}
