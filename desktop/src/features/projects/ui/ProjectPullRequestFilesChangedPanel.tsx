import { FileDiff, Files, GitCommitHorizontal, Search } from "lucide-react";
import * as React from "react";

import type { ProjectPullRequest } from "@/features/projects/hooks";
import { cn } from "@/shared/lib/cn";
import type { ProjectRepoDiff, ProjectRepoDiffFile } from "@/shared/api/types";

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

function parseHunkHeader(line: string) {
  const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
  if (!match) return null;
  return { oldLine: Number(match[1]), newLine: Number(match[2]) };
}

function diffRows(file: ProjectRepoDiffFile): DiffRow[] {
  const seen = new Map<string, number>();
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
    .map((line) => {
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
      const rawKey = `${row.type}:${row.oldLine ?? ""}:${row.newLine ?? ""}:${row.content}`;
      const count = seen.get(rawKey) ?? 0;
      seen.set(rawKey, count + 1);
      return { ...row, key: `${file.path}:${count}:${rawKey}` };
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
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return null;
}

function DiffPreview({ file }: { file: ProjectRepoDiffFile }) {
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
      {rows.map((row) => (
        <div
          className={cn(
            "grid min-h-5 grid-cols-[3rem_3rem_1.5rem_minmax(0,1fr)]",
            diffLineClassName(row.type),
          )}
          key={row.key}
        >
          <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
            {row.oldLine ?? " "}
          </span>
          <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
            {row.newLine ?? " "}
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
      ))}
    </div>
  );
}

export function ProjectPullRequestFilesChangedPanel({
  error,
  diff,
  isLoading,
  pullRequest,
}: {
  error: unknown;
  diff: ProjectRepoDiff | null | undefined;
  isLoading: boolean;
  pullRequest: ProjectPullRequest | null;
}) {
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
      <div className="rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
        Loading changed files…
      </div>
    );
  }

  if (error) {
    const message = errorMessage(error);
    return (
      <div className="space-y-1 rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
        <p>Could not load changed files for this pull request.</p>
        {message ? (
          <p className="font-mono text-xs text-muted-foreground/80">
            {message}
          </p>
        ) : null}
      </div>
    );
  }

  if (!pullRequest || files.length === 0) {
    return (
      <div className="rounded-xl border border-border/50 bg-card/60 p-6 text-center text-sm text-muted-foreground">
        No changed files are available for this pull request yet.
      </div>
    );
  }

  return (
    <div className="grid min-h-0 overflow-hidden rounded-xl border border-border/50 bg-card/60 lg:grid-cols-[17rem_minmax(0,1fr)]">
      <aside className="border-border/50 border-b bg-background/30 lg:border-r lg:border-b-0">
        <div className="space-y-3 p-3">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Files className="h-3.5 w-3.5" />
            <span>{files.length} changed files</span>
          </div>
          <label className="flex h-8 items-center gap-2 rounded-lg border border-border/60 bg-background/70 px-2 text-xs text-muted-foreground">
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
          {filteredFiles.map((file) => (
            <button
              className={cn(
                "flex w-full min-w-0 items-center gap-2 px-3 py-1.5 text-left text-xs text-muted-foreground hover:bg-muted/35 hover:text-foreground focus-visible:bg-muted/35 focus-visible:outline-hidden",
                selectedPath === file.path && "bg-muted/45 text-foreground",
              )}
              key={file.path}
              onClick={() => setSelectedPath(file.path)}
              type="button"
            >
              <FileDiff className="h-3.5 w-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate">{file.path}</span>
            </button>
          ))}
        </nav>
      </aside>

      <section className="min-w-0">
        <div className="flex min-h-12 flex-wrap items-center justify-between gap-3 border-border/50 border-b bg-background/30 px-4 py-2 text-xs text-muted-foreground">
          <div className="flex min-w-0 items-center gap-2">
            <GitCommitHorizontal className="h-3.5 w-3.5" />
            <span className="truncate">
              {pullRequest.title} · {pullRequest.commit?.slice(0, 7) ?? "PR"}
            </span>
          </div>
          <div className="flex items-center gap-3">
            <span>{files.length} files changed</span>
            <span className="text-green-500">+{stats.additions}</span>
            <span className="text-destructive">-{stats.deletions}</span>
          </div>
        </div>

        <div className="p-3">
          {selectedFile ? (
            <article className="overflow-hidden rounded-lg border border-border/50 bg-background/45">
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
              <DiffPreview file={selectedFile} />
            </article>
          ) : (
            <div className="rounded-lg border border-border/50 bg-background/45 p-4 text-sm text-muted-foreground">
              No files match this filter.
            </div>
          )}
        </div>
      </section>
    </div>
  );
}
