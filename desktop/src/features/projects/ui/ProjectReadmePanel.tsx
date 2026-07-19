import { BookOpen } from "lucide-react";

import type { ProjectRepoFile } from "@/features/projects/hooks";
import { Markdown, SyntaxHighlightedCode } from "@/shared/ui/markdown";
import {
  baseName,
  formatLastChangedAt,
  languageForPath,
} from "./ProjectRepositoryPanel";
import {
  type RepoSourceHeaderControls,
  RepoSourceDropdown,
  RepoSyncActionButton,
  RepositoryBranchDropdown,
} from "./ProjectRepositorySource";

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

export function ReadmePanel({
  file,
  sourceControls,
}: {
  file: ProjectRepoFile | null;
  /** Branch picker + remote/local toggle rendered in the panel header. */
  sourceControls?: RepoSourceHeaderControls;
}) {
  // Two header rows, mirroring the files panel: controls on top, then the
  // file identity row.
  const header = (
    <>
      {sourceControls ? (
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
      ) : null}
      <div className="flex min-h-10 items-center gap-2 border-border/50 border-b bg-muted/20 px-4">
        <BookOpen className="h-4 w-4 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
          {file ? baseName(file.path) : "README"}
        </span>
        {file ? (
          <span className="hidden shrink-0 text-2xs text-muted-foreground sm:block">
            Last changed {formatLastChangedAt(file.lastChangedAt)}
          </span>
        ) : null}
      </div>
    </>
  );

  if (!file?.previewContent) {
    return (
      <section className="overflow-hidden">
        {sourceControls ? header : null}
        <div className="p-6 text-sm text-muted-foreground">
          Add a README to this repository to describe setup, usage, and project
          context.
        </div>
      </section>
    );
  }

  const language = languageForPath(file.path);
  const isMarkdown = /\.(?:md|markdown|mdx)$/i.test(file.path);
  const readmeContent = isMarkdown
    ? normalizeReadmeMarkdown(file.previewContent)
    : file.previewContent;

  return (
    <section className="overflow-hidden">
      {header}
      <div className="p-4">
        {isMarkdown ? (
          <Markdown
            className="text-sm"
            content={readmeContent}
            interactive={false}
          />
        ) : language ? (
          <pre className="overflow-x-auto bg-muted/40 p-4">
            <SyntaxHighlightedCode
              className="text-xs leading-relaxed"
              code={file.previewContent}
              language={language}
            />
          </pre>
        ) : (
          <pre className="overflow-x-auto bg-muted/40 p-4">
            <code className="block min-w-full whitespace-pre font-mono text-xs leading-relaxed text-foreground">
              {file.previewContent}
            </code>
          </pre>
        )}
      </div>
    </section>
  );
}
