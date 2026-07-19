import {
  Braces,
  ChevronRight,
  CodeXml,
  Database,
  FileArchive,
  FileAudio,
  FileCode2,
  FileCog,
  FileDiff,
  FileImage,
  FileJson,
  FileLock2,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  FolderGit2,
  Package,
  Settings,
  Terminal,
} from "lucide-react";
import * as React from "react";

import type {
  ProjectRepoFile,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { SyntaxHighlightedCode } from "@/shared/ui/markdown";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  type RepoSourceHeaderControls,
  RepoSourceDropdown,
  RepoSyncActionButton,
  RepositoryBranchDropdown,
} from "./ProjectRepositorySource";

function relativeCommitTime(createdAt: number) {
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

function pluralize(count: number, singular: string) {
  return `${count} ${singular}${count === 1 ? "" : "s"}`;
}

export function formatLastChangedAt(timestamp: number | null) {
  if (!timestamp) return "—";
  return new Date(timestamp * 1_000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function formatFileSize(size: number | null) {
  if (size === null) return "—";
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function normalizeAuthorLookupValue(value: string | null | undefined) {
  return value?.trim().toLowerCase() ?? "";
}

type CommitAuthorProfile = {
  avatarUrl: string | null;
  displayName: string | null;
  isAgent?: boolean;
  nip05Handle?: string | null;
  ownerPubkey?: string | null;
};

function profileMatchesCommitAuthor(
  commit: ProjectRepoFile["latestCommit"],
  profile: CommitAuthorProfile,
  pubkey?: string,
) {
  if (!commit) return false;

  const authorName = normalizeAuthorLookupValue(commit.authorName);
  const authorEmail = normalizeAuthorLookupValue(commit.authorEmail);
  if (!authorName && !authorEmail) return false;

  const candidates = [
    pubkey,
    profile.displayName,
    profile.nip05Handle,
    profile.ownerPubkey,
  ].map(normalizeAuthorLookupValue);

  return candidates.includes(authorName) || candidates.includes(authorEmail);
}

function profileForCommitAuthor(
  commit: ProjectRepoFile["latestCommit"],
  profiles: UserProfileLookup | undefined,
) {
  if (!commit || !profiles) return null;

  for (const [pubkey, profile] of Object.entries(profiles)) {
    if (profileMatchesCommitAuthor(commit, profile, pubkey)) {
      return profile;
    }
  }

  return null;
}

function searchedProfileForCommitAuthor(
  commit: ProjectRepoFile["latestCommit"],
  users: UserSearchResult[] | undefined,
) {
  if (!commit || !users?.length) return null;

  return (
    users.find((user) =>
      profileMatchesCommitAuthor(commit, user, user.pubkey),
    ) ?? users[0]
  );
}

function commitAuthorLabel(
  commit: ProjectRepoFile["latestCommit"],
  profile?: CommitAuthorProfile | null,
) {
  if (!commit) return "";

  return (
    profile?.displayName?.trim() ||
    profile?.nip05Handle?.trim() ||
    commit.authorName
  );
}

function RepositoryCommitCell({
  commit,
  profiles,
}: {
  commit: ProjectRepoFile["latestCommit"];
  profiles?: UserProfileLookup;
}) {
  if (!commit) return <span className="text-muted-foreground">—</span>;

  return (
    <p className="truncate text-sm text-foreground">
      {commit.subject}
      <span className="text-muted-foreground">
        {" "}
        · {commitAuthorLabel(commit, profileForCommitAuthor(commit, profiles))}
      </span>
    </p>
  );
}

export function baseName(path: string) {
  return path.split("/").pop() || path;
}

const FILE_LANGUAGE_BY_EXTENSION: Record<string, string> = {
  c: "c",
  cc: "cpp",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  dart: "dart",
  go: "go",
  h: "c",
  hpp: "cpp",
  html: "html",
  java: "java",
  js: "javascript",
  json: "json",
  jsx: "jsx",
  kt: "kotlin",
  kts: "kotlin",
  md: "markdown",
  mjs: "javascript",
  mts: "typescript",
  py: "python",
  rb: "ruby",
  rs: "rust",
  sh: "bash",
  sql: "sql",
  swift: "swift",
  toml: "toml",
  ts: "typescript",
  tsx: "tsx",
  yaml: "yaml",
  yml: "yaml",
  zig: "zig",
};

export function languageForPath(path: string) {
  const fileName = baseName(path).toLowerCase();
  if (fileName === "dockerfile") return "dockerfile";
  if (fileName === "makefile") return "make";
  const extension = fileName.split(".").pop();
  return extension ? FILE_LANGUAGE_BY_EXTENSION[extension] : undefined;
}

type FileIconVisual = {
  Icon: React.ComponentType<{ className?: string }>;
  className: string;
  containerClassName: string;
};

const CODE_EXTENSIONS = new Set([
  "c",
  "cc",
  "cpp",
  "cs",
  "dart",
  "go",
  "java",
  "js",
  "jsx",
  "kt",
  "kts",
  "mjs",
  "mts",
  "py",
  "rb",
  "rs",
  "swift",
  "ts",
  "tsx",
  "zig",
]);
const IMAGE_EXTENSIONS = new Set([
  "gif",
  "heic",
  "jpeg",
  "jpg",
  "png",
  "svg",
  "webp",
]);
const ARCHIVE_EXTENSIONS = new Set([
  "7z",
  "bz2",
  "gz",
  "rar",
  "tar",
  "tgz",
  "zip",
]);
const AUDIO_EXTENSIONS = new Set(["aac", "flac", "m4a", "mp3", "ogg", "wav"]);
const VIDEO_EXTENSIONS = new Set(["avi", "m4v", "mov", "mp4", "webm"]);
const SPREADSHEET_EXTENSIONS = new Set(["csv", "ods", "tsv", "xls", "xlsx"]);
const TEXT_EXTENSIONS = new Set(["md", "mdx", "rst", "txt"]);

function extensionForPath(path: string) {
  const name = baseName(path).toLowerCase();
  if (!name.includes(".")) return "";
  return name.split(".").pop() ?? "";
}

function fileIconVisual(entry: RepositoryFileEntry): FileIconVisual {
  if (entry.type === "directory") {
    return {
      Icon: FolderGit2,
      className: "fill-sky-500/25 text-sky-500",
      containerClassName: "bg-sky-500/15",
    };
  }

  const name = entry.name.toLowerCase();
  const extension = extensionForPath(entry.name);

  if (
    name === "dockerfile" ||
    name === "containerfile" ||
    name === "package.json"
  ) {
    return {
      Icon: Package,
      className: "fill-orange-500/20 text-orange-500",
      containerClassName: "bg-orange-500/15",
    };
  }

  if (name.includes("lock") || extension === "pem" || extension === "key") {
    return {
      Icon: FileLock2,
      className: "fill-amber-500/20 text-amber-500",
      containerClassName: "bg-amber-500/15",
    };
  }

  if (extension === "json") {
    return {
      Icon: FileJson,
      className: "fill-yellow-500/20 text-yellow-500",
      containerClassName: "bg-yellow-500/15",
    };
  }

  if (
    ["yaml", "yml", "toml", "ini", "conf", "config", "env"].includes(extension)
  ) {
    return {
      Icon: Settings,
      className: "fill-zinc-500/20 text-zinc-500",
      containerClassName: "bg-zinc-500/15",
    };
  }

  if (["html", "xml"].includes(extension)) {
    return {
      Icon: CodeXml,
      className: "fill-rose-500/20 text-rose-500",
      containerClassName: "bg-rose-500/15",
    };
  }

  if (extension === "css") {
    return {
      Icon: Braces,
      className: "fill-violet-500/20 text-violet-500",
      containerClassName: "bg-violet-500/15",
    };
  }

  if (CODE_EXTENSIONS.has(extension)) {
    return {
      Icon: FileCode2,
      className: "fill-blue-500/20 text-blue-500",
      containerClassName: "bg-blue-500/15",
    };
  }

  if (IMAGE_EXTENSIONS.has(extension)) {
    return {
      Icon: FileImage,
      className: "fill-pink-500/20 text-pink-500",
      containerClassName: "bg-pink-500/15",
    };
  }

  if (ARCHIVE_EXTENSIONS.has(extension)) {
    return {
      Icon: FileArchive,
      className: "fill-orange-500/20 text-orange-500",
      containerClassName: "bg-orange-500/15",
    };
  }

  if (AUDIO_EXTENSIONS.has(extension)) {
    return {
      Icon: FileAudio,
      className: "fill-purple-500/20 text-purple-500",
      containerClassName: "bg-purple-500/15",
    };
  }

  if (VIDEO_EXTENSIONS.has(extension)) {
    return {
      Icon: FileVideo,
      className: "fill-red-500/20 text-red-500",
      containerClassName: "bg-red-500/15",
    };
  }

  if (SPREADSHEET_EXTENSIONS.has(extension)) {
    return {
      Icon: FileSpreadsheet,
      className: "fill-emerald-500/20 text-emerald-500",
      containerClassName: "bg-emerald-500/15",
    };
  }

  if (extension === "sql" || extension === "db" || extension === "sqlite") {
    return {
      Icon: Database,
      className: "fill-cyan-500/20 text-cyan-500",
      containerClassName: "bg-cyan-500/15",
    };
  }

  if (["bash", "fish", "sh", "zsh"].includes(extension)) {
    return {
      Icon: Terminal,
      className: "fill-lime-500/20 text-lime-500",
      containerClassName: "bg-lime-500/15",
    };
  }

  if (TEXT_EXTENSIONS.has(extension)) {
    return {
      Icon: FileText,
      className: "fill-slate-500/20 text-slate-500",
      containerClassName: "bg-slate-500/15",
    };
  }

  if (extension === "pdf") {
    return {
      Icon: FileType,
      className: "fill-red-500/20 text-red-500",
      containerClassName: "bg-red-500/15",
    };
  }

  return {
    Icon: FileCog,
    className: "fill-muted-foreground/20 text-muted-foreground",
    containerClassName: "bg-muted/70",
  };
}

function RepositoryEntryIcon({ entry }: { entry: RepositoryFileEntry }) {
  const visual = fileIconVisual(entry);
  const Icon = visual.Icon;

  return (
    <span
      className={cn(
        "flex h-6 w-6 shrink-0 items-center justify-center",
        visual.containerClassName,
      )}
    >
      <Icon className={cn("h-4 w-4", visual.className)} />
    </span>
  );
}

type RepositoryFileEntry = {
  file?: ProjectRepoFile;
  fileCount?: number;
  lastChangedAt: number | null;
  latestCommit: ProjectRepoFile["latestCommit"];
  name: string;
  path: string;
  size: number | null;
  type: "directory" | "file";
};

function repositoryEntries(
  files: ProjectRepoFile[],
  currentPath: string,
): RepositoryFileEntry[] {
  const directories = new Map<string, RepositoryFileEntry>();
  const entries: RepositoryFileEntry[] = [];
  const prefix = currentPath ? `${currentPath}/` : "";

  for (const file of files) {
    if (currentPath && !file.path.startsWith(prefix)) continue;

    const relativePath = currentPath
      ? file.path.slice(prefix.length)
      : file.path;
    const [name, ...rest] = relativePath.split("/");
    if (!name) continue;

    if (rest.length > 0) {
      const path = currentPath ? `${currentPath}/${name}` : name;
      const existing = directories.get(path);
      if (existing) {
        existing.fileCount = (existing.fileCount ?? 0) + 1;
        if ((file.lastChangedAt ?? 0) > (existing.lastChangedAt ?? 0)) {
          existing.lastChangedAt = file.lastChangedAt;
          existing.latestCommit = file.latestCommit;
        }
      } else {
        directories.set(path, {
          fileCount: 1,
          lastChangedAt: file.lastChangedAt,
          latestCommit: file.latestCommit,
          name,
          path,
          size: null,
          type: "directory",
        });
      }
      continue;
    }

    entries.push({
      file,
      lastChangedAt: file.lastChangedAt,
      latestCommit: file.latestCommit,
      name,
      path: file.path,
      size: file.size,
      type: "file",
    });
  }

  return [...directories.values(), ...entries].sort((left, right) => {
    if (left.type !== right.type) return left.type === "directory" ? -1 : 1;
    return left.name.localeCompare(right.name);
  });
}

function openRepositoryEntry(
  entry: RepositoryFileEntry,
  setCurrentPath: (path: string) => void,
  setSelectedFile: (file: ProjectRepoFile) => void,
) {
  if (entry.type === "directory") {
    setCurrentPath(entry.path);
    return;
  }

  if (entry.file) setSelectedFile(entry.file);
}

function handleRepositoryEntryKeyDown(
  event: React.KeyboardEvent<HTMLTableRowElement>,
  onOpen: () => void,
) {
  if (event.key !== "Enter" && event.key !== " ") return;

  event.preventDefault();
  onOpen();
}

function BreadcrumbButton({
  children,
  onClick,
}: {
  children: React.ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      className="truncate rounded-md px-1.5 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
      onClick={onClick}
      type="button"
    >
      {children}
    </button>
  );
}

function FileContentPanel({
  file,
  onOpenPath,
}: {
  file: ProjectRepoFile;
  onOpenPath: (path: string) => void;
}) {
  const language = languageForPath(file.path);
  const pathSegments = file.path.split("/").filter(Boolean);
  const fileName = pathSegments[pathSegments.length - 1] ?? file.path;
  const directorySegments = pathSegments.slice(0, -1);

  return (
    <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
      <div className="flex min-h-14 items-center gap-1 border-border/50 border-b bg-muted/20 px-3 py-3">
        <BreadcrumbButton onClick={() => onOpenPath("")}>
          Files
        </BreadcrumbButton>
        {directorySegments.map((segment, index) => {
          const nextPath = directorySegments.slice(0, index + 1).join("/");
          return (
            <React.Fragment key={nextPath}>
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
              <BreadcrumbButton onClick={() => onOpenPath(nextPath)}>
                {segment}
              </BreadcrumbButton>
            </React.Fragment>
          );
        })}
        <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        <FileDiff className="h-4 w-4 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate px-1.5 py-1 font-mono text-xs text-foreground">
          {fileName}
        </span>
        <div className="hidden shrink-0 items-center gap-3 text-2xs text-muted-foreground sm:flex">
          <span>Last changed {formatLastChangedAt(file.lastChangedAt)}</span>
          <span>{formatFileSize(file.size)}</span>
        </div>
        <span className="shrink-0 text-2xs text-muted-foreground sm:hidden">
          {formatFileSize(file.size)}
        </span>
      </div>
      <div className="border-border/50 border-b bg-muted/10 px-4 py-2 text-2xs text-muted-foreground sm:hidden">
        Last changed {formatLastChangedAt(file.lastChangedAt)}
      </div>
      {file.previewContent ? (
        <pre className="max-h-[36rem] overflow-auto bg-background/60 p-4">
          {language ? (
            <SyntaxHighlightedCode
              className="text-xs leading-relaxed"
              code={file.previewContent}
              language={language}
            />
          ) : (
            <code className="block min-w-full whitespace-pre font-mono text-xs leading-relaxed text-foreground">
              {file.previewContent}
            </code>
          )}
        </pre>
      ) : (
        <div className="p-6 text-sm text-muted-foreground">
          Preview unavailable for this file. Large and binary files only show
          metadata.
        </div>
      )}
    </div>
  );
}

export function RepositoryFilesPanel({
  files,
  snapshot,
  isLoading,
  error,
  profiles,
  fallbackAuthorPubkey,
  sourceControls,
}: {
  files: ProjectRepoFile[];
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
  profiles?: UserProfileLookup;
  fallbackAuthorPubkey?: string;
  /** Branch picker + remote/local toggle rendered in the panel header. */
  sourceControls?: RepoSourceHeaderControls;
}) {
  const [currentPath, setCurrentPath] = React.useState("");
  const [selectedFile, setSelectedFile] =
    React.useState<ProjectRepoFile | null>(null);
  const entries = React.useMemo(
    () => repositoryEntries(files, currentPath),
    [currentPath, files],
  );
  const visibleEntries = entries.slice(0, 200);
  const latestCommit = snapshot?.latestCommit ?? null;
  const knownLatestCommitProfile = React.useMemo(
    () => profileForCommitAuthor(latestCommit, profiles),
    [latestCommit, profiles],
  );
  const fallbackLatestCommitProfile = fallbackAuthorPubkey
    ? (profiles?.[normalizePubkey(fallbackAuthorPubkey)] ?? null)
    : null;
  const authorSearchQuery = useUserSearchQuery(
    latestCommit?.authorEmail || latestCommit?.authorName || "",
    {
      enabled:
        Boolean(latestCommit) &&
        !knownLatestCommitProfile?.avatarUrl &&
        !fallbackLatestCommitProfile?.avatarUrl,
      limit: 3,
    },
  );
  const searchedLatestCommitProfile = React.useMemo(
    () => searchedProfileForCommitAuthor(latestCommit, authorSearchQuery.data),
    [authorSearchQuery.data, latestCommit],
  );
  const latestCommitProfile =
    knownLatestCommitProfile ??
    searchedLatestCommitProfile ??
    fallbackLatestCommitProfile;
  const latestCommitAuthorLabel = commitAuthorLabel(
    latestCommit,
    latestCommitProfile,
  );
  const pathSegments = currentPath ? currentPath.split("/") : [];

  const filesKey = React.useMemo(
    () => files.map((file) => file.path).join("\0"),
    [files],
  );

  React.useEffect(() => {
    if (!filesKey) return;
    setCurrentPath("");
    setSelectedFile(null);
  }, [filesKey]);

  // Loading/error/empty states keep the header controls visible — the
  // remote/local toggle must stay reachable when one source fails to load.
  const stateMessage = isLoading
    ? "Loading repository files…"
    : error
      ? "Could not load the repository file tree."
      : files.length === 0
        ? "No files have been pushed yet."
        : null;
  if (stateMessage) {
    if (!sourceControls) {
      return (
        <div className="rounded-xl border border-border/60 bg-card p-4 text-sm text-muted-foreground">
          {stateMessage}
        </div>
      );
    }
    return (
      <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
        <div className="flex min-h-14 min-w-0 items-center gap-1 border-border/50 border-b px-3 py-3">
          <RepoSourceDropdown controls={sourceControls} />
          <RepositoryBranchDropdown
            branch={sourceControls.branch}
            branchOptions={sourceControls.branchOptions}
            compact
            onBranchChange={sourceControls.onBranchChange}
          />
          <div className="ml-auto flex shrink-0 items-center">
            <RepoSyncActionButton controls={sourceControls} />
          </div>
        </div>
        <div className="p-4 text-sm text-muted-foreground">{stateMessage}</div>
      </div>
    );
  }

  if (selectedFile) {
    return (
      <FileContentPanel
        file={selectedFile}
        onOpenPath={(path) => {
          setSelectedFile(null);
          setCurrentPath(path);
        }}
      />
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/60 bg-card">
      <div className="flex min-h-14 min-w-0 items-center gap-1 border-border/50 border-b px-3 py-3">
        {sourceControls ? (
          <>
            <RepoSourceDropdown controls={sourceControls} />
            <RepositoryBranchDropdown
              branch={sourceControls.branch}
              branchOptions={sourceControls.branchOptions}
              compact
              onBranchChange={sourceControls.onBranchChange}
            />
          </>
        ) : (
          <BreadcrumbButton onClick={() => setCurrentPath("")}>
            Files
          </BreadcrumbButton>
        )}
        {sourceControls && pathSegments.length > 0 ? (
          <>
            <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
            <BreadcrumbButton onClick={() => setCurrentPath("")}>
              Files
            </BreadcrumbButton>
          </>
        ) : null}
        {pathSegments.map((segment, index) => {
          const nextPath = pathSegments.slice(0, index + 1).join("/");
          return (
            <React.Fragment key={nextPath}>
              <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
              <BreadcrumbButton onClick={() => setCurrentPath(nextPath)}>
                {segment}
              </BreadcrumbButton>
            </React.Fragment>
          );
        })}
        {sourceControls ? (
          <div className="ml-auto flex shrink-0 items-center">
            <RepoSyncActionButton controls={sourceControls} />
          </div>
        ) : null}
      </div>

      <div className="overflow-x-auto">
        <table className="w-full caption-bottom text-sm">
          <thead>
            <tr className="border-border/50 border-b bg-muted/20">
              <th className="px-4 py-3 text-left font-normal" colSpan={3}>
                {latestCommit ? (
                  <div className="flex min-w-0 items-center justify-between gap-3 text-sm">
                    <div className="flex min-w-0 items-center gap-2">
                      <UserAvatar
                        accent={latestCommitProfile?.isAgent === true}
                        avatarUrl={latestCommitProfile?.avatarUrl ?? null}
                        displayName={latestCommitAuthorLabel}
                        size="sm"
                      />
                      <p className="min-w-0 flex-1 truncate text-foreground">
                        <span className="font-semibold">
                          {latestCommitAuthorLabel}
                        </span>
                        <span className="text-muted-foreground">
                          {" "}
                          committed
                        </span>
                        <span className="font-medium">
                          {" "}
                          {latestCommit.subject}
                        </span>
                        <code
                          className="mx-1.5 rounded-md bg-background/60 px-1.5 py-0.5 text-2xs text-muted-foreground"
                          title={latestCommit.hash}
                        >
                          {latestCommit.shortHash}
                        </code>
                        <span className="text-muted-foreground">
                          · {pluralize(files.length, "file")}
                        </span>
                      </p>
                    </div>
                    <time
                      className="shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground"
                      dateTime={new Date(
                        latestCommit.timestamp * 1_000,
                      ).toISOString()}
                    >
                      {relativeCommitTime(latestCommit.timestamp)}
                    </time>
                  </div>
                ) : (
                  <p className="truncate text-sm font-medium text-foreground">
                    Repository files · {files.length} tracked files
                  </p>
                )}
              </th>
            </tr>
          </thead>
          <tbody>
            {visibleEntries.map((entry, index) => {
              const latestCommit = entry.latestCommit;
              const rowIsLast = index === visibleEntries.length - 1;
              const openEntry = () =>
                openRepositoryEntry(entry, setCurrentPath, setSelectedFile);

              return (
                <tr
                  aria-label={`Open ${entry.type} ${entry.name}`}
                  className={cn(
                    "cursor-pointer transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:outline-hidden",
                    !rowIsLast && "border-border/50 border-b",
                  )}
                  key={`${entry.type}:${entry.path}`}
                  onClick={openEntry}
                  onKeyDown={(event) =>
                    handleRepositoryEntryKeyDown(event, openEntry)
                  }
                  tabIndex={0}
                >
                  <td className="min-w-52 p-3 align-middle">
                    <div className="flex min-w-0 items-center gap-2">
                      <RepositoryEntryIcon entry={entry} />
                      <span className="truncate font-medium text-foreground">
                        {entry.name}
                      </span>
                    </div>
                  </td>
                  <td className="max-w-96 p-3 align-middle">
                    <RepositoryCommitCell
                      commit={latestCommit}
                      profiles={profiles}
                    />
                  </td>
                  <td className="w-36 whitespace-nowrap p-3 text-right align-middle text-muted-foreground">
                    {latestCommit ? (
                      <time
                        dateTime={new Date(
                          latestCommit.timestamp * 1_000,
                        ).toISOString()}
                      >
                        {relativeCommitTime(latestCommit.timestamp)}
                      </time>
                    ) : (
                      "—"
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {entries.length > 200 ? (
        <p className="border-border/50 border-t px-4 py-3 text-2xs text-muted-foreground">
          Showing the first 200 entries in this folder. Open a folder to narrow
          the list.
        </p>
      ) : null}
    </div>
  );
}
