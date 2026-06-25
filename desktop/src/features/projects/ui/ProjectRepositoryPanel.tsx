import {
  ArrowLeft,
  BookOpen,
  ChevronRight,
  FileDiff,
  FolderGit2,
} from "lucide-react";
import * as React from "react";

import type {
  ProjectRepoFile,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import { Button } from "@/shared/ui/button";
import { Markdown, SyntaxHighlightedCode } from "@/shared/ui/markdown";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function formatLastChangedAt(timestamp: number | null) {
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

function baseName(path: string) {
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

function languageForPath(path: string) {
  const fileName = baseName(path).toLowerCase();
  if (fileName === "dockerfile") return "dockerfile";
  if (fileName === "makefile") return "make";
  const extension = fileName.split(".").pop();
  return extension ? FILE_LANGUAGE_BY_EXTENSION[extension] : undefined;
}

type RepositoryFileEntry = {
  file?: ProjectRepoFile;
  fileCount?: number;
  lastChangedAt: number | null;
  name: string;
  path: string;
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
        existing.lastChangedAt = Math.max(
          existing.lastChangedAt ?? 0,
          file.lastChangedAt ?? 0,
        );
      } else {
        directories.set(path, {
          fileCount: 1,
          lastChangedAt: file.lastChangedAt,
          name,
          path,
          type: "directory",
        });
      }
      continue;
    }

    entries.push({
      file,
      lastChangedAt: file.lastChangedAt,
      name,
      path: file.path,
      type: "file",
    });
  }

  return [...directories.values(), ...entries].sort((left, right) => {
    if (left.type !== right.type) return left.type === "directory" ? -1 : 1;
    return left.name.localeCompare(right.name);
  });
}

export function findReadmeFile(files: ProjectRepoFile[]) {
  const readmes = files.filter((file) =>
    /^readme(?:\.(?:md|markdown|mdx|txt))?$/i.test(baseName(file.path)),
  );

  return readmes.find((file) => !file.path.includes("/")) ?? readmes[0] ?? null;
}

function decodeHtmlEntities(value: string) {
  return value
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"')
    .replace(/&#39;/g, "'");
}

function htmlInlineToMarkdown(value: string): string {
  return decodeHtmlEntities(value)
    .replace(/<br\s*\/?\s*>/gi, "\n")
    .replace(/<img\b([^>]*)>/gi, (_match: string, attrs: string) => {
      const src = attrs.match(/\bsrc=["']([^"']+)["']/i)?.[1];
      const alt = attrs.match(/\balt=["']([^"']*)["']/i)?.[1] ?? "";
      return src ? `![${alt}](${src})` : "";
    })
    .replace(
      /<a\b[^>]*\bhref=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi,
      (_match: string, href: string, label: string) =>
        `[${htmlInlineToMarkdown(label).trim()}](${href})`,
    )
    .replace(/<(strong|b)\b[^>]*>([\s\S]*?)<\/\1>/gi, "**$2**")
    .replace(/<(em|i)\b[^>]*>([\s\S]*?)<\/\1>/gi, "*$2*")
    .replace(/<code\b[^>]*>([\s\S]*?)<\/code>/gi, "`$1`")
    .replace(/<sub\b[^>]*>([\s\S]*?)<\/sub>/gi, "$1")
    .replace(/<span\b[^>]*>([\s\S]*?)<\/span>/gi, "$1")
    .replace(/<[^>]+>/g, "")
    .trim();
}

function normalizeReadmeMarkdown(content: string) {
  return content
    .replace(
      /<h([1-6])\b[^>]*>([\s\S]*?)<\/h\1>/gi,
      (_match, depth: string, value: string) =>
        `${"#".repeat(Number(depth))} ${htmlInlineToMarkdown(value)}\n\n`,
    )
    .replace(
      /<p\b[^>]*>([\s\S]*?)<\/p>/gi,
      (_match, value: string) => `${htmlInlineToMarkdown(value)}\n\n`,
    )
    .replace(
      /<div\b[^>]*>([\s\S]*?)<\/div>/gi,
      (_match, value: string) => `${htmlInlineToMarkdown(value)}\n\n`,
    )
    .replace(
      /<center\b[^>]*>([\s\S]*?)<\/center>/gi,
      (_match, value: string) => `${htmlInlineToMarkdown(value)}\n\n`,
    )
    .replace(/\n{3,}/g, "\n\n")
    .trim();
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
  onBack,
}: {
  file: ProjectRepoFile;
  onBack: () => void;
}) {
  const language = languageForPath(file.path);

  return (
    <div className="overflow-hidden rounded-xl border border-border/50 bg-card/60">
      <div className="flex min-h-10 items-center gap-2 border-border/50 border-b bg-muted/20 px-4">
        <Button className="h-7 px-2" onClick={onBack} size="sm" variant="ghost">
          <ArrowLeft className="h-3.5 w-3.5" />
          Back
        </Button>
        <FileDiff className="h-4 w-4 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">
          {file.path}
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
}: {
  files: ProjectRepoFile[];
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
}) {
  const [currentPath, setCurrentPath] = React.useState("");
  const [selectedFile, setSelectedFile] =
    React.useState<ProjectRepoFile | null>(null);
  const entries = React.useMemo(
    () => repositoryEntries(files, currentPath),
    [currentPath, files],
  );
  const latestCommit = snapshot?.latestCommit ?? null;
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

  if (isLoading) {
    return (
      <div className="rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
        Loading repository files…
      </div>
    );
  }

  if (error) {
    return (
      <div className="rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
        Could not load the repository file tree.
      </div>
    );
  }

  if (files.length === 0) {
    return (
      <div className="rounded-xl border border-border/50 bg-card/60 p-6 text-center text-sm text-muted-foreground">
        No files have been pushed yet.
      </div>
    );
  }

  if (selectedFile) {
    return (
      <FileContentPanel
        file={selectedFile}
        onBack={() => setSelectedFile(null)}
      />
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/50 bg-card/60">
      <div className="flex flex-col gap-2 border-border/50 border-b bg-muted/20 px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          <p className="truncate text-sm font-medium text-foreground">
            {latestCommit?.subject ?? "Repository files"}
          </p>
          {latestCommit ? (
            <p className="truncate text-xs text-muted-foreground">
              {latestCommit.authorName} committed{" "}
              {compactDate(latestCommit.timestamp)}
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              {files.length} tracked files
            </p>
          )}
        </div>
        {latestCommit ? (
          <code className="w-fit shrink-0 rounded-md bg-background/60 px-2 py-1 text-xs text-muted-foreground">
            {latestCommit.shortHash}
          </code>
        ) : null}
      </div>

      <div className="flex min-h-10 min-w-0 items-center gap-1 border-border/50 border-b px-3">
        <BreadcrumbButton onClick={() => setCurrentPath("")}>
          Files
        </BreadcrumbButton>
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
      </div>

      <div className="hidden grid-cols-[minmax(0,1fr)_9rem_5rem] gap-3 border-border/50 border-b bg-muted/10 px-4 py-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground sm:grid">
        <span>Name</span>
        <span>Last changed</span>
        <span className="text-right">Size</span>
      </div>

      <div className="divide-y divide-border/50">
        {currentPath ? (
          <button
            className="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-4 py-2.5 text-left text-sm transition-colors hover:bg-muted/30 sm:grid-cols-[minmax(0,1fr)_9rem_5rem]"
            onClick={() => {
              const parent = currentPath.split("/").slice(0, -1).join("/");
              setCurrentPath(parent);
            }}
            type="button"
          >
            <div className="flex min-w-0 items-center gap-2">
              <FolderGit2 className="h-4 w-4 shrink-0 text-muted-foreground" />
              <span className="truncate font-medium text-muted-foreground">
                ..
              </span>
            </div>
            <span className="hidden shrink-0 text-xs text-muted-foreground sm:block" />
            <span className="shrink-0 text-xs text-muted-foreground">
              Parent
            </span>
          </button>
        ) : null}
        {entries.slice(0, 200).map((entry) => {
          const Icon = entry.type === "directory" ? FolderGit2 : FileDiff;
          const meta =
            entry.type === "directory"
              ? `${entry.fileCount ?? 0} files`
              : formatFileSize(entry.file?.size ?? null);
          const lastChanged = formatLastChangedAt(entry.lastChangedAt);

          return (
            <button
              className="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 px-4 py-2.5 text-left text-sm transition-colors hover:bg-muted/30 sm:grid-cols-[minmax(0,1fr)_9rem_5rem]"
              key={`${entry.type}:${entry.path}`}
              onClick={() => {
                if (entry.type === "directory") {
                  setCurrentPath(entry.path);
                  return;
                }
                if (entry.file) setSelectedFile(entry.file);
              }}
              type="button"
            >
              <div className="flex min-w-0 items-center gap-2">
                <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
                <span className="truncate font-medium text-foreground">
                  {entry.name}
                </span>
              </div>
              <span className="hidden shrink-0 text-xs text-muted-foreground sm:block">
                {lastChanged}
              </span>
              <span className="shrink-0 text-right text-xs text-muted-foreground">
                {meta}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function ReadmePanel({ file }: { file: ProjectRepoFile | null }) {
  if (!file?.previewContent) {
    return (
      <div className="rounded-xl border border-border/50 bg-card/60 p-6 text-sm text-muted-foreground">
        Add a README to this repository to describe setup, usage, and project
        context.
      </div>
    );
  }

  const language = languageForPath(file.path);
  const isMarkdown = /\.(?:md|markdown|mdx)$/i.test(file.path);
  const readmeContent = isMarkdown
    ? normalizeReadmeMarkdown(file.previewContent)
    : file.previewContent;

  return (
    <section className="overflow-hidden rounded-xl border border-border/50 bg-card/60">
      <div className="flex min-h-10 items-center gap-2 border-border/50 border-b bg-muted/20 px-4">
        <BookOpen className="h-4 w-4 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {baseName(file.path)}
        </span>
        <span className="hidden shrink-0 text-2xs text-muted-foreground sm:block">
          Last changed {formatLastChangedAt(file.lastChangedAt)}
        </span>
      </div>
      <div className="p-4">
        {isMarkdown ? (
          <Markdown
            className="text-sm"
            content={readmeContent}
            interactive={false}
          />
        ) : language ? (
          <pre className="overflow-x-auto rounded-lg bg-muted/40 p-4">
            <SyntaxHighlightedCode
              className="text-xs leading-relaxed"
              code={file.previewContent}
              language={language}
            />
          </pre>
        ) : (
          <pre className="overflow-x-auto rounded-lg bg-muted/40 p-4">
            <code className="block min-w-full whitespace-pre font-mono text-xs leading-relaxed text-foreground">
              {file.previewContent}
            </code>
          </pre>
        )}
      </div>
    </section>
  );
}
