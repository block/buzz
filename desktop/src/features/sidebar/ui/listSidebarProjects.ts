import type { Project } from "@/features/projects/projectModels";
import { isProjectOwnedByCurrentUser } from "@/features/projects/lib/projectsViewHelpers";

const SIDEBAR_PROJECTS_FILTER_KEY = "buzz.sidebar.projects.filter";
const SIDEBAR_PROJECTS_SORT_KEY = "buzz.sidebar.projects.sort";
const SIDEBAR_PROJECTS_EXPANDED_KEY = "buzz.sidebar.projects.expanded";

export type SidebarProjectsFilter = "added" | "owned";
export type SidebarProjectsSort = "name" | "created";
export type SidebarProjectExpansionState = Record<string, boolean>;

function expandedProjectsStorageKey(
  relayOrigin: string | null,
  currentPubkey?: string,
) {
  return `${SIDEBAR_PROJECTS_EXPANDED_KEY}:${encodeURIComponent(relayOrigin ?? "unknown")}:${currentPubkey ?? "anonymous"}`;
}

export function readSidebarProjectExpansion(
  relayOrigin: string | null,
  currentPubkey?: string,
): SidebarProjectExpansionState {
  try {
    const value = globalThis.localStorage?.getItem(
      expandedProjectsStorageKey(relayOrigin, currentPubkey),
    );
    if (!value) return {};
    const parsed: unknown = JSON.parse(value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed))
      return {};
    return Object.fromEntries(
      Object.entries(parsed).filter((entry): entry is [string, boolean] => {
        return typeof entry[1] === "boolean";
      }),
    );
  } catch {
    return {};
  }
}

export function writeSidebarProjectExpansion(
  expansion: SidebarProjectExpansionState,
  relayOrigin: string | null,
  currentPubkey?: string,
) {
  try {
    globalThis.localStorage?.setItem(
      expandedProjectsStorageKey(relayOrigin, currentPubkey),
      JSON.stringify(expansion),
    );
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function selectedProjectRouteId(pathname: string): string | undefined {
  if (!pathname.startsWith("/projects/")) return undefined;
  const raw = pathname.slice("/projects/".length).split("/")[0];
  if (!raw) return undefined;
  try {
    return decodeURIComponent(raw);
  } catch {
    return raw;
  }
}

export function readSidebarProjectsFilter(): SidebarProjectsFilter {
  try {
    const value = globalThis.localStorage?.getItem(SIDEBAR_PROJECTS_FILTER_KEY);
    return value === "owned" ? "owned" : "added";
  } catch {
    return "added";
  }
}

export function writeSidebarProjectsFilter(filter: SidebarProjectsFilter) {
  try {
    globalThis.localStorage?.setItem(SIDEBAR_PROJECTS_FILTER_KEY, filter);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function readSidebarProjectsSort(): SidebarProjectsSort {
  try {
    const value = globalThis.localStorage?.getItem(SIDEBAR_PROJECTS_SORT_KEY);
    return value === "created" ? "created" : "name";
  } catch {
    return "name";
  }
}

export function writeSidebarProjectsSort(sort: SidebarProjectsSort) {
  try {
    globalThis.localStorage?.setItem(SIDEBAR_PROJECTS_SORT_KEY, sort);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function listSidebarProjects({
  addedProjectAddresses,
  currentPubkey,
  filter,
  projects,
  sort,
}: {
  addedProjectAddresses: ReadonlySet<string>;
  currentPubkey: string | undefined;
  filter: SidebarProjectsFilter;
  projects: readonly Project[];
  sort: SidebarProjectsSort;
}): Project[] {
  return [...projects]
    .filter(
      (project) =>
        addedProjectAddresses.has(project.projectAddress) &&
        (filter !== "owned" ||
          isProjectOwnedByCurrentUser(project, currentPubkey)),
    )
    .sort((left, right) => {
      if (sort === "created") {
        return (
          right.createdAt - left.createdAt ||
          left.name.localeCompare(right.name)
        );
      }
      return left.name.localeCompare(right.name);
    });
}
