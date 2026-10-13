import { BookOpen, MessageCircle } from "lucide-react";

import type { ProjectRepoFile } from "@/features/projects/hooks";
import type { ProjectRepoUnavailableReason } from "@/features/projects/lib/projectRepoAvailability";
import { formatLastChangedAt } from "@/features/projects/lib/projectsViewHelpers";
import { Button } from "@/shared/ui/button";
import { BuzzLoadingState } from "@/shared/ui/BuzzLoadingState";
import { Markdown, SyntaxHighlightedCode } from "@/shared/ui/markdown";
import { baseName, languageForPath } from "./ProjectRepositoryPanel";
import { GitHubRepoStateRecovery } from "./GitHubRepoStateRecovery";
import {
  type RepositoryFileContentSource,
  useRepositoryFileContent,
} from "./useRepositoryFileContent";
import {
  type RepoSourceHeaderControls,
  RepoSourceDropdown,
  RepoSyncActionButton,
  RepositoryBranchDropdown,
} from "./ProjectRepositorySource";
import { ProjectRepositoryUnavailableState } from "./ProjectRepositoryUnavailableState";
import { ProjectPanelState } from "./ProjectPanelState";

export function findReadmeFile(files: ProjectRepoFile[]) {
  const readmes = files.filter((file) =>
    /^readme(?:\.(?:md|markdown|mdx|txt))?$/i.test(baseName(file.path)),
  );
  const previewFiles = files.filter((file) => Boolean(file.previewContent));

  return (
    readmes.find((file) => !file.path.includes("/")) ??
    readmes[0] ??
    (previewFiles.length === 1 ? previewFiles[0] : null)
  );
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
  accessChannelId,
  file,
  fileContentSource,
  gitDataState,
  externalHost,
  hideHeader,
  ownerAvatarUrl,
  ownerIsAgent,
  ownerName,
  sourceControls,
  unavailableReason,
}: {
  /** `buzz-channel` binding of the repository, for access-restricted copy. */
  accessChannelId?: string | null;
  file: ProjectRepoFile | null;
  fileContentSource?: RepositoryFileContentSource;
  gitDataState: "checking" | "available" | "empty" | "unavailable";
  /** Git host name when the repository is not hosted on the relay. */
  externalHost?: string;
  /**
   * Skip the header rows entirely — the workspace layout renders the source
   * controls and last-changed timestamp itself.
   */
  hideHeader?: boolean;
  ownerAvatarUrl?: string | null;
  ownerIsAgent?: boolean;
  ownerName?: string;
  unavailableReason?: ProjectRepoUnavailableReason;
  /** Branch picker + remote/local toggle rendered in the panel header. */
  sourceControls?: RepoSourceHeaderControls;
}) {
  const fileContent = useRepositoryFileContent(file, fileContentSource);
  // Two header rows, mirroring the files panel: controls on top, then the
  // file identity row.
  const header = hideHeader ? null : (
    <>
      {sourceControls ? (
        <>
          <div className="flex min-h-14 min-w-0 items-center gap-1 border-border/50 border-b px-3 py-3">
            <RepoSourceDropdown controls={sourceControls} />
            <RepositoryBranchDropdown
              branch={sourceControls.branch}
              branchOptions={sourceControls.branchOptions}
              createBranchDisabled={sourceControls.createBranchDisabled}
              createBranchTitle={sourceControls.createBranchTitle}
              deleteBranchDisabled={sourceControls.deleteBranchDisabled}
              deleteBranchTitle={sourceControls.deleteBranchTitle}
              onBranchChange={sourceControls.onBranchChange}
              onCreateBranch={sourceControls.onCreateBranch}
              onDeleteBranch={sourceControls.onDeleteBranch}
              onTagChange={sourceControls.onTagChange}
              selectedTag={sourceControls.selectedTag}
              tagOptions={sourceControls.tagOptions}
            />
            <div className="ml-auto flex shrink-0 items-center">
              <RepoSyncActionButton controls={sourceControls} />
            </div>
          </div>
          {sourceControls.showGithubStateRecovery ? (
            <div className="border-border/50 border-b px-3 py-2">
              <GitHubRepoStateRecovery
                error={sourceControls.stateError}
                onRetry={sourceControls.onRetryState}
              />
            </div>
          ) : null}
        </>
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

  if (gitDataState === "checking") {
    return (
      <section className="overflow-hidden">
        {header}
        <BuzzLoadingState label="Loading repository" />
      </section>
    );
  }

  if (gitDataState === "unavailable") {
    return (
      <section className="overflow-hidden">
        <ProjectRepositoryUnavailableState
          accessChannelId={accessChannelId}
          externalHost={externalHost}
          onAskForAccess={sourceControls?.onAskForAccess}
          onRetry={sourceControls?.onFetch}
          ownerAvatarUrl={ownerAvatarUrl}
          ownerIsAgent={ownerIsAgent}
          ownerName={ownerName}
          reason={unavailableReason}
          retryPending={sourceControls?.fetchPending}
        />
      </section>
    );
  }

  if (fileContent.isLoading) {
    return (
      <section className="overflow-hidden">
        {header}
        <BuzzLoadingState label="Loading README" />
      </section>
    );
  }

  if (!file || !fileContent.content) {
    const loadError = Boolean(fileContent.error);
    const emptyRepository = gitDataState === "empty";
    return (
      <section className="flex min-h-0 flex-1 flex-col overflow-hidden">
        {header}
        <ProjectPanelState
          action={
            sourceControls?.onAskForAccess ? (
              <Button onClick={sourceControls.onAskForAccess} size="sm">
                <MessageCircle className="h-4 w-4" />
                Chat with an agent
              </Button>
            ) : undefined
          }
          description={
            loadError
              ? "Refresh the repository or ask an agent to investigate."
              : emptyRepository
                ? "Ask an agent to create the initial codebase or connect an existing repository."
                : "Add a README to describe setup, usage, and project context."
          }
          error={loadError}
          panel={false}
          title={
            loadError
              ? "Could not load the README"
              : emptyRepository
                ? "No files have been pushed yet"
                : "No README yet"
          }
        />
      </section>
    );
  }

  const language = languageForPath(file.path);
  const isMarkdown = /\.(?:md|markdown|mdx)$/i.test(file.path);
  const readmeContent = isMarkdown
    ? normalizeReadmeMarkdown(fileContent.content)
    : fileContent.content;

  return (
    <section className="overflow-hidden">
      {header}
      <div className="min-w-0 px-8 py-6">
        {isMarkdown ? (
          <Markdown
            blockCode
            className="text-sm"
            content={readmeContent}
            hardLineBreaks={false}
            interactive={false}
          />
        ) : language ? (
          <pre className="overflow-x-auto bg-muted/40 p-4">
            <SyntaxHighlightedCode
              className="text-xs leading-relaxed"
              code={fileContent.content}
              language={language}
            />
          </pre>
        ) : (
          <pre className="overflow-x-auto bg-muted/40 p-4">
            <code className="block min-w-full whitespace-pre font-mono text-xs leading-relaxed text-foreground">
              {fileContent.content}
            </code>
          </pre>
        )}
      </div>
    </section>
  );
}
