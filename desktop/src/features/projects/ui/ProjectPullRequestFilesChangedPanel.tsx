import {
  Braces,
  CodeXml,
  Database,
  FileArchive,
  FileAudio,
  FileCode2,
  FileCog,
  FileDiff,
  Files,
  FileImage,
  FileJson,
  FileLock2,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  FolderGit2,
  GitCommitHorizontal,
  MessageSquarePlus,
  Package,
  Search,
  Settings,
  Terminal,
  type LucideIcon,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import {
  type Project,
  type ProjectPullRequest,
  type ProjectPullRequestCommentAnchor,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import type { ProjectPullRequestComment } from "@/features/projects/projectPullRequests.mjs";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import type { ProjectRepoDiff, ProjectRepoDiffFile } from "@/shared/api/types";
import { ProjectPullRequestInlineCommentThread } from "./ProjectPullRequestInlineComments";

function fileName(path: string) {
  return path.split("/").pop() || path;
}

function directoryName(path: string) {
  const segments = path.split("/");
  segments.pop();
  return segments.join("/");
}

type DiffRow = {
  content: string;
  key: string;
  newLine: number | null;
  oldLine: number | null;
  type: "add" | "context" | "delete" | "hunk";
};

type InlineCommentControls = {
  activeAnchor: ProjectPullRequestCommentAnchor | null;
  comments: ProjectPullRequestComment[];
  isSending: boolean;
  onCancel: () => void;
  onStart: (anchor: ProjectPullRequestCommentAnchor) => void;
  onSubmit: (
    anchor: ProjectPullRequestCommentAnchor,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<unknown>;
  profiles?: UserProfileLookup;
};

type FileTreeNode = {
  children: Map<string, FileTreeNode>;
  file: ProjectRepoDiffFile | null;
  name: string;
  path: string;
};

type ChangedFileIconVisual = {
  Icon: LucideIcon;
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
  "h",
  "hpp",
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
  "avif",
  "gif",
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

function createFileTreeNode(name: string, path: string): FileTreeNode {
  return { children: new Map(), file: null, name, path };
}

function buildFileTree(files: ProjectRepoDiffFile[]) {
  const root = createFileTreeNode("", "");
  for (const file of files) {
    const segments = file.path.split("/").filter(Boolean);
    let node = root;
    segments.forEach((segment, index) => {
      const path = segments.slice(0, index + 1).join("/");
      let child = node.children.get(segment);
      if (!child) {
        child = createFileTreeNode(segment, path);
        node.children.set(segment, child);
      }
      node = child;
    });
    node.file = file;
  }
  return root;
}

function sortedFileTreeChildren(node: FileTreeNode) {
  return [...node.children.values()].sort((left, right) => {
    if (Boolean(left.file) !== Boolean(right.file)) {
      return left.file ? 1 : -1;
    }
    return left.name.localeCompare(right.name);
  });
}

function extensionForPath(path: string) {
  const name = fileName(path).toLowerCase();
  if (!name.includes(".")) return "";
  return name.split(".").pop() ?? "";
}

function changedFileIconVisual(path: string): ChangedFileIconVisual {
  const name = fileName(path).toLowerCase();
  const extension = extensionForPath(path);

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

function ChangedFileTreeIcon({ path }: { path: string }) {
  const visual = changedFileIconVisual(path);
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

function ChangedFolderTreeIcon() {
  return (
    <span className="flex h-6 w-6 shrink-0 items-center justify-center bg-sky-500/15">
      <FolderGit2 className="h-4 w-4 fill-sky-500/25 text-sky-500" />
    </span>
  );
}

function parseHunkHeader(line: string) {
  const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
  if (!match) return null;
  return { oldLine: Number(match[1]), newLine: Number(match[2]) };
}

function diffRows(file: ProjectRepoDiffFile): DiffRow[] {
  let oldLine = 0;
  let newLine = 0;
  return file.patch
    .trimEnd()
    .split("\n")
    .filter(
      (line) =>
        !line.startsWith("diff --git ") &&
        !line.startsWith("index ") &&
        !line.startsWith("--- ") &&
        !line.startsWith("+++ "),
    )
    .map((line, index) => {
      const hunk = parseHunkHeader(line);
      let row: Omit<DiffRow, "key">;
      if (hunk) {
        oldLine = hunk.oldLine;
        newLine = hunk.newLine;
        row = { content: line, oldLine: null, newLine: null, type: "hunk" };
      } else if (line.startsWith("+")) {
        row = {
          content: line.slice(1),
          oldLine: null,
          newLine: newLine++,
          type: "add",
        };
      } else if (line.startsWith("-")) {
        row = {
          content: line.slice(1),
          oldLine: oldLine++,
          newLine: null,
          type: "delete",
        };
      } else {
        row = {
          content: line.startsWith(" ") ? line.slice(1) : line,
          oldLine: oldLine++,
          newLine: newLine++,
          type: "context",
        };
      }
      // Rows are computed once per patch in source order, so a positional
      // index is a stable, cheap key.
      return { ...row, key: `${file.path}:${index}` };
    });
}

function diffLineClassName(type: DiffRow["type"]) {
  if (type === "add") return "border-green-500/10 border-l-2 bg-green-500/10";
  if (type === "delete")
    return "border-destructive/10 border-l-2 bg-destructive/10";
  if (type === "hunk") return "bg-sky-500/10 text-sky-500";
  return "border-transparent border-l-2";
}

function linePrefix(type: DiffRow["type"]) {
  if (type === "add") return "+";
  if (type === "delete") return "-";
  return " ";
}

function commentAnchorForRow(
  file: ProjectRepoDiffFile,
  row: DiffRow,
): ProjectPullRequestCommentAnchor | null {
  if (row.type === "hunk") return null;
  const side = row.type === "delete" ? "old" : "new";
  const line = side === "old" ? row.oldLine : row.newLine;
  return line ? { line, path: file.path, side } : null;
}

function anchorsEqual(
  left: ProjectPullRequestCommentAnchor | null,
  right: ProjectPullRequestCommentAnchor | null,
) {
  return Boolean(
    left &&
      right &&
      left.line === right.line &&
      left.path === right.path &&
      left.side === right.side,
  );
}

function fileAdditions(file: ProjectRepoDiffFile) {
  return file.additions;
}

function changedFileStats(diff: ProjectRepoDiff | null | undefined) {
  return {
    additions: diff?.additions ?? 0,
    deletions: diff?.deletions ?? 0,
  };
}

function errorMessage(error: unknown) {
  const message =
    error instanceof Error
      ? error.message
      : typeof error === "string"
        ? error
        : null;
  if (!message) return null;
  // Backend errors can carry raw git stderr with temp-dir paths; strip
  // absolute paths so the UI doesn't leak local filesystem details.
  return message.replace(/(^|[\s'"`])(?:[A-Za-z]:)?[\\/][^\s'"`]+/g, "$1…");
}

function DiffPreview({
  file,
  inlineComments,
}: {
  file: ProjectRepoDiffFile;
  inlineComments?: InlineCommentControls;
}) {
  const rows = diffRows(file);
  if (rows.length === 0) {
    return (
      <div className="bg-muted/20 px-4 py-4 text-sm text-muted-foreground">
        No textual diff is available for this file.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto bg-background/70 font-mono text-xs leading-5">
      {file.truncated ? (
        <div className="border-border/40 border-b bg-amber-500/10 px-4 py-2 text-amber-600 dark:text-amber-400">
          Large diff truncated — showing the first {rows.length} lines. Use a
          local checkout to review the full change.
        </div>
      ) : null}
      {rows.map((row) => {
        const anchor = commentAnchorForRow(file, row);
        const comments =
          anchor && inlineComments
            ? inlineComments.comments.filter(
                (comment) =>
                  comment.anchor && anchorsEqual(comment.anchor, anchor),
              )
            : [];
        const isActive = anchorsEqual(
          inlineComments?.activeAnchor ?? null,
          anchor,
        );
        return (
          <React.Fragment key={row.key}>
            <div
              className={cn(
                "group grid min-h-5 grid-cols-[3rem_3rem_2rem_1.5rem_minmax(0,1fr)]",
                diffLineClassName(row.type),
              )}
              data-line={anchor?.line}
              data-path={anchor?.path}
              data-side={anchor?.side}
              data-testid={anchor ? "project-diff-line" : undefined}
            >
              <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
                {row.oldLine ?? " "}
              </span>
              <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
                {row.newLine ?? " "}
              </span>
              <span className="flex select-none items-center justify-center">
                {anchor && inlineComments ? (
                  <button
                    aria-label={`Comment on ${anchor.path} ${anchor.side} line ${anchor.line}`}
                    className={cn(
                      "flex h-5 w-5 items-center justify-center rounded text-muted-foreground opacity-0 hover:bg-primary hover:text-primary-foreground focus-visible:opacity-100 focus-visible:outline-hidden group-hover:opacity-100",
                      (comments.length > 0 || isActive) && "opacity-100",
                    )}
                    data-testid="project-diff-add-comment"
                    onClick={() => inlineComments.onStart(anchor)}
                    title="Add line comment"
                    type="button"
                  >
                    <MessageSquarePlus className="h-3.5 w-3.5" />
                  </button>
                ) : null}
              </span>
              <span
                className={cn(
                  "select-none px-2",
                  row.type === "add" && "text-green-500",
                  row.type === "delete" && "text-destructive",
                )}
              >
                {linePrefix(row.type)}
              </span>
              <code className="min-w-0 whitespace-pre pr-3 text-foreground">
                {row.content || " "}
              </code>
            </div>
            {anchor && inlineComments ? (
              <ProjectPullRequestInlineCommentThread
                activeAnchor={isActive ? anchor : null}
                comments={comments}
                isSending={inlineComments.isSending}
                onCancel={inlineComments.onCancel}
                onSubmit={(content, mentionPubkeys, mediaTags) =>
                  inlineComments.onSubmit(
                    anchor,
                    content,
                    mentionPubkeys,
                    mediaTags,
                  )
                }
                profiles={inlineComments.profiles}
              />
            ) : null}
          </React.Fragment>
        );
      })}
    </div>
  );
}

function FileTreeItems({
  node,
  onSelect,
  selectedPath,
  depth = 0,
}: {
  node: FileTreeNode;
  onSelect: (path: string) => void;
  selectedPath: string | null;
  depth?: number;
}) {
  return sortedFileTreeChildren(node).map((child) => {
    if (child.file) {
      return (
        <button
          className={cn(
            "flex w-full min-w-0 items-center gap-2 py-1.5 pr-3 text-left text-xs text-muted-foreground hover:bg-muted/35 hover:text-foreground focus-visible:bg-muted/35 focus-visible:outline-hidden",
            selectedPath === child.file.path && "bg-muted/45 text-foreground",
          )}
          key={child.path}
          onClick={() => onSelect(child.file?.path ?? child.path)}
          style={{ paddingLeft: `${0.75 + depth * 0.9}rem` }}
          type="button"
        >
          <ChangedFileTreeIcon path={child.file.path} />
          <span className="min-w-0 flex-1 truncate">{child.name}</span>
        </button>
      );
    }

    return (
      <div key={child.path}>
        <div
          className="flex min-w-0 items-center gap-2 py-1.5 pr-3 text-xs font-medium text-muted-foreground"
          style={{ paddingLeft: `${0.75 + depth * 0.9}rem` }}
        >
          <ChangedFolderTreeIcon />
          <span className="min-w-0 flex-1 truncate">{child.name}</span>
        </div>
        <FileTreeItems
          depth={depth + 1}
          node={child}
          onSelect={onSelect}
          selectedPath={selectedPath}
        />
      </div>
    );
  });
}

export function ProjectPullRequestFilesChangedPanel({
  error,
  diff,
  isLoading,
  profiles,
  project,
  pullRequest,
}: {
  error: unknown;
  diff: ProjectRepoDiff | null | undefined;
  isLoading: boolean;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest | null;
}) {
  const [activeAnchor, setActiveAnchor] =
    React.useState<ProjectPullRequestCommentAnchor | null>(null);
  const { isPending: isPostingComment, mutateAsync: postComment } =
    useCreateProjectPullRequestCommentMutation(project);
  const inlineComments = React.useMemo(
    () =>
      pullRequest?.comments.filter(
        (comment) =>
          comment.anchor && comment.inlineCommentStatus === "current",
      ) ?? [],
    [pullRequest],
  );
  const handleSubmit = React.useCallback(
    async (
      anchor: ProjectPullRequestCommentAnchor,
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      if (!pullRequest) throw new Error("No pull request selected.");
      try {
        await postComment({
          anchor,
          content,
          mediaTags,
          mentionPubkeys,
          pullRequest,
        });
        setActiveAnchor(null);
        toast.success("Line comment posted.");
      } catch (error) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Failed to post line comment.",
        );
        throw error;
      }
    },
    [postComment, pullRequest],
  );

  return (
    <ProjectDiffFilesPanel
      diff={pullRequest ? diff : null}
      embedded
      error={error}
      headerLabel={
        pullRequest
          ? `${pullRequest.title} · ${pullRequest.commit?.slice(0, 7) ?? "PR"}`
          : ""
      }
      inlineComments={
        pullRequest
          ? {
              activeAnchor,
              comments: inlineComments,
              isSending: isPostingComment,
              onCancel: () => setActiveAnchor(null),
              onStart: setActiveAnchor,
              onSubmit: handleSubmit,
              profiles,
            }
          : undefined
      }
      isLoading={isLoading}
      subjectLabel="pull request"
    />
  );
}

export function ProjectDiffFilesPanel({
  error,
  diff,
  isLoading,
  embedded = false,
  headerLabel,
  inlineComments,
  subjectLabel,
}: {
  error: unknown;
  diff: ProjectRepoDiff | null | undefined;
  isLoading: boolean;
  /** Render without an outer border, for nesting inside an existing card. */
  embedded?: boolean;
  headerLabel: string;
  inlineComments?: InlineCommentControls;
  subjectLabel: string;
}) {
  const outerBorderClass = embedded
    ? ""
    : "rounded-xl border border-border/60 bg-card";
  const [query, setQuery] = React.useState("");
  const [selectedPath, setSelectedPath] = React.useState<string | null>(null);
  const files = diff?.files ?? [];
  const filteredFiles = React.useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return files;
    return files.filter((file) =>
      file.path.toLowerCase().includes(normalizedQuery),
    );
  }, [files, query]);
  const stats = React.useMemo(() => changedFileStats(diff), [diff]);
  const fileTree = React.useMemo(
    () => buildFileTree(filteredFiles),
    [filteredFiles],
  );
  const selectedFile =
    filteredFiles.find((file) => file.path === selectedPath) ??
    filteredFiles[0] ??
    null;

  React.useEffect(() => {
    if (filteredFiles.length === 0) {
      setSelectedPath(null);
      return;
    }
    if (
      !selectedPath ||
      !filteredFiles.some((file) => file.path === selectedPath)
    ) {
      setSelectedPath(filteredFiles[0].path);
    }
  }, [filteredFiles, selectedPath]);

  if (isLoading) {
    return (
      <div
        className={cn("p-4 text-sm text-muted-foreground", outerBorderClass)}
      >
        Loading changed files…
      </div>
    );
  }

  if (error) {
    const message = errorMessage(error);
    return (
      <div
        className={cn(
          "space-y-1 p-4 text-sm text-muted-foreground",
          outerBorderClass,
        )}
      >
        <p>Could not load changed files for this {subjectLabel}.</p>
        {message ? (
          <p className="font-mono text-xs text-muted-foreground/80">
            {message}
          </p>
        ) : null}
      </div>
    );
  }

  if (files.length === 0) {
    return (
      <div
        className={cn(
          "p-6 text-center text-sm text-muted-foreground",
          outerBorderClass,
        )}
      >
        No changed files are available for this {subjectLabel} yet.
      </div>
    );
  }

  return (
    <div
      className={cn(
        "grid min-h-0 overflow-hidden lg:grid-cols-[17rem_minmax(0,1fr)]",
        outerBorderClass,
      )}
    >
      <aside className="border-border/50 border-b bg-background/30 lg:border-r lg:border-b-0">
        <div className="space-y-3 p-3">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Files className="h-3.5 w-3.5" />
            <span>{files.length} changed files</span>
          </div>
          <label className="flex h-8 items-center gap-2 border border-border/60 bg-background/70 px-2 text-xs text-muted-foreground">
            <Search className="h-3.5 w-3.5" />
            <input
              className="min-w-0 flex-1 bg-transparent text-foreground outline-hidden placeholder:text-muted-foreground"
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Filter files…"
              value={query}
            />
          </label>
        </div>
        <nav className="max-h-96 overflow-auto border-border/50 border-t py-1">
          <FileTreeItems
            node={fileTree}
            onSelect={setSelectedPath}
            selectedPath={selectedPath}
          />
        </nav>
      </aside>

      <section className="min-w-0">
        <div className="flex min-h-12 flex-wrap items-center justify-between gap-3 border-border/50 border-b bg-background/30 px-4 py-2 text-xs text-muted-foreground">
          <div className="flex min-w-0 items-center gap-2">
            <GitCommitHorizontal className="h-3.5 w-3.5" />
            <span className="truncate">{headerLabel}</span>
          </div>
          <div className="flex items-center gap-3">
            <span>{files.length} files changed</span>
            <span className="text-green-500">+{stats.additions}</span>
            <span className="text-destructive">-{stats.deletions}</span>
          </div>
        </div>

        <div className="p-3">
          {selectedFile ? (
            <article className="overflow-hidden border border-border/60 bg-background/45">
              <header className="flex min-h-10 items-center justify-between gap-3 border-border/50 border-b bg-muted/20 px-3 text-xs">
                <div className="flex min-w-0 items-center gap-2">
                  <FileDiff className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate font-medium text-foreground">
                    {fileName(selectedFile.path)}
                  </span>
                  {directoryName(selectedFile.path) ? (
                    <span className="truncate text-muted-foreground">
                      {directoryName(selectedFile.path)}
                    </span>
                  ) : null}
                </div>
                <div className="flex shrink-0 items-center gap-2 text-muted-foreground">
                  <span
                    className={cn(
                      fileAdditions(selectedFile) > 0 && "text-green-500",
                    )}
                  >
                    +{fileAdditions(selectedFile)}
                  </span>
                  <span className="text-destructive">
                    -{selectedFile.deletions}
                  </span>
                </div>
              </header>
              <DiffPreview
                file={selectedFile}
                inlineComments={inlineComments}
              />
            </article>
          ) : (
            <div className="border border-border/60 bg-background/45 p-4 text-sm text-muted-foreground">
              No files match this filter.
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
