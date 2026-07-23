import type {
  Project,
  ProjectActivitySummary,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ProjectsViewMode = "grid" | "list";
export type ProjectsRepositoryScope = "all" | "mine" | "local";
export type ProjectsWorkItemScope = "all" | "mine";
export type ProjectsFilter =
  | "all"
  | "mine"
  | "local"
  | "repositories"
  | "prs"
  | "issues"
  | "agents"
  | "users";
export type ProjectsSort = "updated" | "created" | "name";

const PROJECTS_VIEW_MODE_STORAGE_KEY = "buzz.projects.viewMode";
const PROJECTS_FILTER_STORAGE_KEY = "buzz.projects.filter";
const PROJECTS_REPOSITORY_SCOPE_STORAGE_KEY = "buzz.projects.repositoryScope";
const PROJECTS_PULL_REQUEST_SCOPE_STORAGE_KEY =
  "buzz.projects.pullRequestScope";
const PROJECTS_ISSUE_SCOPE_STORAGE_KEY = "buzz.projects.issueScope";
const PROJECTS_SORT_STORAGE_KEY = "buzz.projects.sort";

export function readStoredViewMode(): ProjectsViewMode | null {
  try {
    const value = globalThis.localStorage?.getItem(
      PROJECTS_VIEW_MODE_STORAGE_KEY,
    );
    return value === "grid" || value === "list" ? value : null;
  } catch {
    return null;
  }
}

export function writeStoredViewMode(viewMode: ProjectsViewMode) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_VIEW_MODE_STORAGE_KEY, viewMode);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function readStoredFilter(): ProjectsFilter {
  try {
    const value = globalThis.localStorage?.getItem(PROJECTS_FILTER_STORAGE_KEY);
    return value === "mine" ||
      value === "local" ||
      value === "repositories" ||
      value === "prs" ||
      value === "issues" ||
      value === "agents" ||
      value === "users"
      ? value
      : "all";
  } catch {
    return "all";
  }
}

export function writeStoredFilter(filter: ProjectsFilter) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_FILTER_STORAGE_KEY, filter);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function readStoredRepositoryScope(): ProjectsRepositoryScope {
  try {
    const value = globalThis.localStorage?.getItem(
      PROJECTS_REPOSITORY_SCOPE_STORAGE_KEY,
    );
    if (value === "mine" || value === "local") return value;
    const legacyFilter = globalThis.localStorage?.getItem(
      PROJECTS_FILTER_STORAGE_KEY,
    );
    return legacyFilter === "mine" || legacyFilter === "local"
      ? legacyFilter
      : "all";
  } catch {
    return "all";
  }
}

export function writeStoredRepositoryScope(scope: ProjectsRepositoryScope) {
  try {
    globalThis.localStorage?.setItem(
      PROJECTS_REPOSITORY_SCOPE_STORAGE_KEY,
      scope,
    );
  } catch {
    // Persistence is best-effort; the in-memory filter still works.
  }
}

function readStoredWorkItemScope(key: string): ProjectsWorkItemScope {
  try {
    return globalThis.localStorage?.getItem(key) === "mine" ? "mine" : "all";
  } catch {
    return "all";
  }
}

function writeStoredWorkItemScope(key: string, scope: ProjectsWorkItemScope) {
  try {
    globalThis.localStorage?.setItem(key, scope);
  } catch {
    // Persistence is best-effort; the in-memory filter still works.
  }
}

export function readStoredPullRequestScope(): ProjectsWorkItemScope {
  return readStoredWorkItemScope(PROJECTS_PULL_REQUEST_SCOPE_STORAGE_KEY);
}

export function writeStoredPullRequestScope(scope: ProjectsWorkItemScope) {
  writeStoredWorkItemScope(PROJECTS_PULL_REQUEST_SCOPE_STORAGE_KEY, scope);
}

export function readStoredIssueScope(): ProjectsWorkItemScope {
  return readStoredWorkItemScope(PROJECTS_ISSUE_SCOPE_STORAGE_KEY);
}

export function writeStoredIssueScope(scope: ProjectsWorkItemScope) {
  writeStoredWorkItemScope(PROJECTS_ISSUE_SCOPE_STORAGE_KEY, scope);
}

export function readStoredSort(): ProjectsSort {
  try {
    const value = globalThis.localStorage?.getItem(PROJECTS_SORT_STORAGE_KEY);
    return value === "created" || value === "name" ? value : "updated";
  } catch {
    return "updated";
  }
}

export function writeStoredSort(sort: ProjectsSort) {
  try {
    globalThis.localStorage?.setItem(PROJECTS_SORT_STORAGE_KEY, sort);
  } catch {
    // Persistence is best-effort; the in-memory toggle still works.
  }
}

export function pluralize(
  count: number,
  singular: string,
  plural = `${singular}s`,
) {
  return `${count} ${count === 1 ? singular : plural}`;
}

/**
 * Flatten Markdown source into a clean single-line-friendly plain-text string
 * for compact previews (activity feed, list excerpts). This is intentionally
 * lightweight — it strips the common syntax that would otherwise leak as raw
 * characters (`##`, `**`, backticks, links) rather than rendering rich
 * Markdown, which is inappropriate inside a clamped one/two-line preview.
 */
export function markdownToPlainText(input: string): string {
  return (
    input
      // Fenced code blocks: drop the fences, keep the inner code text.
      .replace(/```[^\n]*\n?/g, "")
      .replace(/```/g, "")
      // Images `![alt](url)` -> alt text.
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      // Links `[text](url)` -> text.
      .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
      // Line-leading markers: headings, blockquotes, list bullets/numbers.
      .replace(/^[ \t]{0,3}(?:#{1,6}|>|[-*+]|\d+[.)])[ \t]+/gm, "")
      // Emphasis: bold/italic/strikethrough — keep the inner text.
      .replace(/(\*\*|__)(.+?)\1/g, "$2")
      .replace(/([*_])(.+?)\1/g, "$2")
      .replace(/~~(.+?)~~/g, "$1")
      // Inline code — keep the inner text.
      .replace(/`([^`]+)`/g, "$1")
  );
}

export function formatCreatedDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function relativeTime(createdAt: number) {
  const elapsedSeconds = Math.max(
    1,
    Math.floor(Date.now() / 1_000 - createdAt),
  );
  const units = [
    { label: "year", seconds: 365 * 24 * 60 * 60 },
    { label: "month", seconds: 30 * 24 * 60 * 60 },
    { label: "week", seconds: 7 * 24 * 60 * 60 },
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "minute", seconds: 60 },
    { label: "second", seconds: 1 },
  ];

  for (const unit of units) {
    const value = Math.floor(elapsedSeconds / unit.seconds);
    if (value >= 1) {
      return `${value} ${unit.label}${value === 1 ? "" : "s"} ago`;
    }
  }

  return "just now";
}

export function formatExactTimestamp(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function projectPeople(
  project: Project,
  summary?: ProjectActivitySummary,
): string[] {
  return [
    ...new Set(
      [
        project.owner,
        ...project.contributors,
        ...(summary?.participantPubkeys ?? []),
      ].map(normalizePubkey),
    ),
  ];
}

export function normalizeRepositoryUrl(url: string) {
  try {
    const parsed = new URL(url);
    const normalizedPath = parsed.pathname
      .replace(/\/+$/, "")
      .replace(/\.git$/i, "")
      .toLowerCase();
    return `${parsed.protocol.toLowerCase()}//${parsed.host.toLowerCase()}${normalizedPath}`;
  } catch {
    return url
      .trim()
      .replace(/\/+$/, "")
      .replace(/\.git$/i, "")
      .toLowerCase();
  }
}

export function getClonePathLabel(project: Project) {
  const cloneUrl = project.cloneUrls[0];
  if (!cloneUrl) return "Clone path pending";

  try {
    const parsed = new URL(cloneUrl);
    return `${parsed.hostname}${parsed.pathname}`;
  } catch {
    return cloneUrl;
  }
}

function repositoryIdentityKey(project: Project) {
  const cloneUrl = project.cloneUrls[0];
  if (cloneUrl) return normalizeRepositoryUrl(cloneUrl);
  return (project.name || project.dtag).trim().toLowerCase();
}

export function uniqueRepositories(projects: Project[]) {
  const seen = new Set<string>();
  return projects.filter((project) => {
    const key = repositoryIdentityKey(project);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

export function getActivityLabel(summary: ProjectActivitySummary | undefined) {
  if (!summary || summary.activityCount === 0) {
    return "No activity yet";
  }

  return [
    pluralize(summary.commitCount, "commit"),
    pluralize(summary.prCount, "PR"),
    pluralize(summary.issueCount, "issue"),
  ].join(" · ");
}

export function getProjectUpdatedAt(
  project: Project,
  summary: ProjectActivitySummary | undefined,
) {
  // A summary with no recorded activity has `updatedAt: 0` — fall back to
  // the announcement's creation time rather than rendering the Unix epoch.
  return summary?.updatedAt || project.createdAt;
}

export function isProjectMine(
  project: Project,
  currentPubkey: string | undefined,
) {
  if (!currentPubkey) return false;
  const normalizedCurrentPubkey = normalizePubkey(currentPubkey);
  return (
    normalizePubkey(project.owner) === normalizedCurrentPubkey ||
    project.contributors.some(
      (pubkey) => normalizePubkey(pubkey) === normalizedCurrentPubkey,
    )
  );
}

export function isProjectOwnedByCurrentUser(
  project: Project,
  currentPubkey: string | undefined,
) {
  return currentPubkey
    ? normalizePubkey(project.owner) === normalizePubkey(currentPubkey)
    : false;
}

export function projectHasAgent(
  project: Project,
  people: string[],
  profiles: UserProfileLookup | undefined,
) {
  const projectPubkeys = [project.owner, ...people];
  return projectPubkeys.some(
    (pubkey) => profiles?.[normalizePubkey(pubkey)]?.isAgent === true,
  );
}

export function projectOwnerIsUser(
  project: Project,
  profiles: UserProfileLookup | undefined,
) {
  return profiles?.[normalizePubkey(project.owner)]?.isAgent !== true;
}
