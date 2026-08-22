import type {
  Project,
  ProjectActivitySummary,
  Repository,
} from "@/features/projects/hooks";
import { hasLocalRepositoryCheckout } from "@/features/projects/lib/projectLocalRepos";
import { projectRepoHostForRepository } from "@/features/projects/lib/projectRepoHost";
import { selectProjectRepository } from "@/features/projects/projectModels";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type ProjectsViewMode = "grid" | "list";
export type ProjectsRepositoryScope =
  | "all"
  | "accessible"
  | "mine"
  | "local"
  | "buzz"
  | "linked";
export type ProjectsWorkItemScope = "all" | "mine" | "assigned";
export type ProjectsFilter =
  | "all"
  | "mine"
  | "local"
  | "projects"
  | "repositories"
  | "channels"
  | "prs"
  | "issues"
  | "agents"
  | "users";
export type ProjectsSort = "updated" | "created" | "name";

export const REPOSITORY_ENTRY_PAGE_SIZE = 200;

export function formatLastChangedAt(timestamp: number | null) {
  if (!timestamp) return "—";
  return new Date(timestamp * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

export function formatFileSize(size: number | null) {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

export function nextRepositoryEntryLimit(current: number, total: number) {
  return Math.min(current + REPOSITORY_ENTRY_PAGE_SIZE, total);
}

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
      value === "projects" ||
      value === "repositories" ||
      value === "channels" ||
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
    if (
      value === "accessible" ||
      value === "mine" ||
      value === "local" ||
      value === "buzz" ||
      value === "linked"
    ) {
      return value;
    }
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

function readStoredWorkItemScope(
  key: string,
  allowed: ProjectsWorkItemScope[],
): ProjectsWorkItemScope {
  try {
    const value = globalThis.localStorage?.getItem(key);
    return allowed.find((scope) => scope === value) ?? "all";
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
  return readStoredWorkItemScope(PROJECTS_PULL_REQUEST_SCOPE_STORAGE_KEY, [
    "mine",
  ]);
}

export function writeStoredPullRequestScope(scope: ProjectsWorkItemScope) {
  writeStoredWorkItemScope(PROJECTS_PULL_REQUEST_SCOPE_STORAGE_KEY, scope);
}

export function readStoredIssueScope(): ProjectsWorkItemScope {
  return readStoredWorkItemScope(PROJECTS_ISSUE_SCOPE_STORAGE_KEY, [
    "mine",
    "assigned",
  ]);
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

/** One-line list subtitle. Empty, whitespace-only, and title-duplicate bodies stay hidden. */
export function listRowDescription(
  value: string | null | undefined,
  title?: string,
): string | undefined {
  const text = markdownToPlainText(value ?? "")
    .replace(/\s+/g, " ")
    .trim();
  if (text.length === 0) return undefined;
  if (
    title &&
    text.localeCompare(title.trim(), undefined, { sensitivity: "accent" }) === 0
  ) {
    return undefined;
  }
  return text;
}

export function formatCreatedDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

export function relativeTime(
  createdAt: number,
  nowSeconds = Math.floor(Date.now() / 1_000),
) {
  const elapsedSeconds = Math.max(1, Math.floor(nowSeconds - createdAt));
  const units = [
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "minute", seconds: 60 },
    { label: "second", seconds: 1 },
  ];

  if (elapsedSeconds >= 7 * 24 * 60 * 60) {
    const createdDate = new Date(createdAt * 1_000);
    const nowDate = new Date(nowSeconds * 1_000);
    return createdDate.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      ...(createdDate.getFullYear() === nowDate.getFullYear()
        ? {}
        : { year: "numeric" }),
    });
  }

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
        ...project.repositories.flatMap((repository) => [
          repository.owner,
          ...repository.contributors,
        ]),
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
  const cloneUrl = selectProjectRepository(project, null)?.cloneUrls[0];
  if (!cloneUrl) return "Clone path pending";

  try {
    const parsed = new URL(cloneUrl);
    return `${parsed.hostname}${parsed.pathname}`;
  } catch {
    return cloneUrl;
  }
}

function repositoryIdentityKey(project: Project) {
  return project.id;
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
    pluralize(summary.prCount, "review"),
    pluralize(summary.issueCount, "task"),
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
    project.repositories.some(
      (repository) =>
        normalizePubkey(repository.owner) === normalizedCurrentPubkey ||
        repository.contributors.some(
          (pubkey) => normalizePubkey(pubkey) === normalizedCurrentPubkey,
        ),
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

export type RepositoryAccessInput = {
  currentPubkey: string | undefined;
  localRepoNames: Set<string>;
  /** `null` while channel memberships are still loading. */
  memberChannelIds: readonly string[] | null;
  relayOrigin: string | null | undefined;
};

/**
 * Whether the viewer can actually read a repository's git data. The relay
 * gates git reads on membership in the repository's bound `buzz-channel`,
 * so a repository is considered accessible when the viewer owns it (owners
 * can repair a missing binding), has a local checkout, the code is hosted
 * externally (no relay ACL applies), or the viewer is a member of the bound
 * channel. While memberships are still loading (`memberChannelIds === null`)
 * channel-bound repositories are kept visible rather than flashing out.
 */
export function isRepositoryAccessibleToViewer(
  repository: Repository,
  input: RepositoryAccessInput,
) {
  if (
    input.currentPubkey &&
    normalizePubkey(repository.owner) === normalizePubkey(input.currentPubkey)
  ) {
    return true;
  }
  if (hasLocalRepositoryCheckout(repository, input.localRepoNames)) {
    return true;
  }
  if (
    projectRepoHostForRepository(repository, input.relayOrigin).kind ===
    "external"
  ) {
    return true;
  }
  if (!repository.channelId) return false;
  if (input.memberChannelIds === null) return true;
  return input.memberChannelIds.includes(repository.channelId);
}

/**
 * A project is accessible when the viewer owns it or can read at least one
 * of its repositories.
 */
export function isProjectAccessibleToViewer(
  project: Project,
  input: RepositoryAccessInput,
) {
  return (
    isProjectOwnedByCurrentUser(project, input.currentPubkey) ||
    project.repositories.some((repository) =>
      isRepositoryAccessibleToViewer(repository, input),
    )
  );
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
