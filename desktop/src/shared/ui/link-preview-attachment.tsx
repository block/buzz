import {
  ExternalLink,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  GitPullRequestDraft,
} from "lucide-react";

import type { SupportedLinkPreview } from "@/shared/lib/linkPreview";
import { cn } from "@/shared/lib/cn";
import {
  githubPullRequestStatus,
  parseGithubPullRequestRef,
  useGithubPullRequestQuery,
} from "@/shared/lib/githubPullRequest";
import {
  Attachment,
  AttachmentActions,
  AttachmentContent,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
} from "@/shared/ui/attachment";

function LinearLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M2.886 4.18A11.982 11.982 0 0 1 11.99 0C18.624 0 24 5.376 24 12.009c0 3.64-1.62 6.903-4.18 9.105L2.887 4.18ZM1.817 5.626l16.556 16.556c-.524.33-1.075.62-1.65.866L.951 7.277c.247-.575.537-1.126.866-1.65ZM.322 9.163l14.515 14.515c-.71.172-1.443.282-2.195.322L0 11.358a12 12 0 0 1 .322-2.195Zm-.17 4.862 9.823 9.824a12.02 12.02 0 0 1-9.824-9.824Z" />
    </svg>
  );
}

function GitHubLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

function GoogleDriveLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M12.01 1.485c-2.082 0-3.754.02-3.743.047.01.02 1.708 3.001 3.774 6.62l3.76 6.574h3.76c2.081 0 3.753-.02 3.742-.047-.005-.02-1.708-3.001-3.775-6.62l-3.76-6.574zm-4.76 1.73a789.828 789.861 0 0 0-3.63 6.319L0 15.868l1.89 3.298 1.885 3.297 3.62-6.335 3.618-6.33-1.88-3.287C8.1 4.704 7.255 3.22 7.25 3.214zm2.259 12.653-.203.348c-.114.198-.96 1.672-1.88 3.287a423.93 423.948 0 0 1-1.698 2.97c-.01.026 3.24.042 7.222.042h7.244l1.796-3.157c.992-1.734 1.85-3.23 1.906-3.323l.104-.167h-7.249z" />
    </svg>
  );
}

function GoogleDocsLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M14.727 6.727H14V0H4.91c-.905 0-1.637.732-1.637 1.636v20.728c0 .904.732 1.636 1.636 1.636h14.182c.904 0 1.636-.732 1.636-1.636V6.727h-6zm-.545 10.455H7.09v-1.364h7.09v1.364zm2.727-3.273H7.091v-1.364h9.818v1.364zm0-3.273H7.091V9.273h9.818v1.363zM14.727 6h6l-6-6v6z" />
    </svg>
  );
}

function GoogleSheetsLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M11.318 12.545H7.91v-1.909h3.41v1.91zM14.728 0v6h6l-6-6zm1.363 10.636h-3.41v1.91h3.41v-1.91zm0 3.273h-3.41v1.91h3.41v-1.91zM20.727 6.5v15.864c0 .904-.732 1.636-1.636 1.636H4.909a1.636 1.636 0 0 1-1.636-1.636V1.636C3.273.732 4.005 0 4.909 0h9.318v6.5h6.5zm-3.273 2.773H6.545v7.909h10.91v-7.91zm-6.136 4.636H7.91v1.91h3.41v-1.91z" />
    </svg>
  );
}

function GoogleSlidesLogo({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      fill="currentColor"
      viewBox="0 0 24 24"
    >
      <path d="M16.09 15.273H7.91v-4.637h8.18v4.637zm1.728-8.523h2.91v15.614c0 .904-.733 1.636-1.637 1.636H4.909a1.636 1.636 0 0 1-1.636-1.636V1.636C3.273.732 4.005 0 4.909 0h9.068v6.75h3.841zm-.363 2.523H6.545v7.363h10.91V9.273zm-2.728-5.979V6h6.001l-6-6v3.294z" />
    </svg>
  );
}

function LinkPreviewLogo({ preview }: { preview: SupportedLinkPreview }) {
  switch (preview.kind) {
    case "github-issue":
    case "github-pull-request":
    case "github-repository":
      return <GitHubLogo className="h-4 w-4" />;
    case "linear-issue":
      return <LinearLogo className="h-4 w-4" />;
    case "google-drive-file":
    case "google-drive-folder":
      return <GoogleDriveLogo className="h-4 w-4" />;
    case "google-docs-document":
      return <GoogleDocsLogo className="h-4 w-4" />;
    case "google-sheets-spreadsheet":
      return <GoogleSheetsLogo className="h-4 w-4" />;
    case "google-slides-presentation":
      return <GoogleSlidesLogo className="h-4 w-4" />;
  }
}

const PR_STATUS_META = {
  open: {
    icon: GitPullRequest,
    label: "Open",
    className: "text-[color:var(--status-added)]",
  },
  draft: {
    icon: GitPullRequestDraft,
    label: "Draft",
    className: "text-muted-foreground",
  },
  merged: {
    icon: GitMerge,
    label: "Merged",
    className: "text-violet-500 dark:text-violet-400",
  },
  closed: {
    icon: GitPullRequestClosed,
    label: "Closed",
    className: "text-[color:var(--status-deleted)]",
  },
} as const;

/**
 * Rich GitHub PR card: live state, diff stats, and files changed from the
 * GitHub API. Falls back to the compact static card while loading or when
 * the fetch fails (rate limit, private repo without a token, offline).
 */
export function GithubPullRequestCard({
  className,
  preview,
}: {
  className?: string;
  preview: SupportedLinkPreview;
}) {
  const ref = parseGithubPullRequestRef(preview.href);
  const query = useGithubPullRequestQuery(ref);
  const info = query.data ?? null;

  if (!ref || !info) {
    return <LinkPreviewAttachment className={className} preview={preview} />;
  }

  const status = githubPullRequestStatus(info);
  const meta = PR_STATUS_META[status];
  const StatusIcon = meta.icon;

  return (
    <Attachment
      className={cn(
        "w-96 max-w-full shrink-0 no-underline shadow-none",
        className,
      )}
      data-link-preview="github-pull-request"
      data-pr-status={status}
    >
      <AttachmentMedia className={cn("link-preview-media", meta.className)}>
        <StatusIcon className="h-4 w-4" />
      </AttachmentMedia>
      <AttachmentContent>
        <div className="flex min-w-0 items-center gap-1 text-xs font-medium leading-4 text-muted-foreground">
          <GitHubLogo className="h-3 w-3 shrink-0" />
          <span className="min-w-0 truncate">
            {ref.owner}/{ref.repo}
          </span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">#{ref.number}</span>
        </div>
        <AttachmentTitle>{info.title || preview.title}</AttachmentTitle>
        <div className="flex min-w-0 items-center gap-2 text-xs leading-4">
          <span className={cn("shrink-0 font-medium", meta.className)}>
            {meta.label}
          </span>
          <span className="shrink-0 font-medium text-[color:var(--status-added)]">
            +{info.additions.toLocaleString()}
          </span>
          <span className="shrink-0 font-medium text-[color:var(--status-deleted)]">
            −{info.deletions.toLocaleString()}
          </span>
          <span className="min-w-0 truncate text-muted-foreground">
            {info.changedFiles.toLocaleString()}{" "}
            {info.changedFiles === 1 ? "file" : "files"} changed
          </span>
        </div>
      </AttachmentContent>
      <AttachmentActions>
        <ExternalLink
          aria-hidden="true"
          className="h-4 w-4 text-muted-foreground opacity-0 transition-opacity group-hover/attachment:opacity-100 group-focus-within/attachment:opacity-100"
        />
      </AttachmentActions>
      <AttachmentTrigger asChild>
        <a
          aria-label={`Open GitHub PR ${ref.owner}/${ref.repo} #${ref.number}: ${info.title || preview.title}`}
          href={preview.href}
          rel="noreferrer"
          target="_blank"
        >
          <span className="sr-only">
            Open GitHub PR {ref.owner}/{ref.repo} #{ref.number}
          </span>
        </a>
      </AttachmentTrigger>
    </Attachment>
  );
}

/**
 * Prominent PR card for agent-authored messages — when the agent reports a
 * pull request it created, the card reads as a piece of delivered work
 * (banner layout, status pill, full-width diff stats) rather than a pasted
 * link. Falls back to the standard rich card without live data.
 */
export function AgentPullRequestCard({
  className,
  preview,
}: {
  className?: string;
  preview: SupportedLinkPreview;
}) {
  const ref = parseGithubPullRequestRef(preview.href);
  const query = useGithubPullRequestQuery(ref);
  const info = query.data ?? null;

  if (!ref || !info) {
    return <GithubPullRequestCard className={className} preview={preview} />;
  }

  const status = githubPullRequestStatus(info);
  const meta = PR_STATUS_META[status];
  const StatusIcon = meta.icon;

  return (
    <Attachment
      className={cn(
        "w-full max-w-xl shrink-0 gap-2 px-4 py-3 no-underline shadow-none",
        className,
      )}
      data-link-preview="github-pull-request-agent"
      data-pr-status={status}
      orientation="vertical"
    >
      <div className="flex w-full min-w-0 items-center gap-1.5 text-xs font-medium leading-4 text-muted-foreground">
        <GitHubLogo className="h-3.5 w-3.5 shrink-0" />
        <span className="min-w-0 truncate">
          {ref.owner}/{ref.repo}
        </span>
        <span aria-hidden="true">·</span>
        <span className="shrink-0">Pull request #{ref.number}</span>
        <span
          className={cn(
            "ml-auto inline-flex shrink-0 items-center gap-1 rounded-full border border-border/70 bg-background px-2 py-0.5 text-2xs font-semibold",
            meta.className,
          )}
        >
          <StatusIcon className="h-3 w-3" />
          {meta.label}
        </span>
      </div>
      <div className="w-full min-w-0 truncate text-sm font-semibold leading-5 text-foreground">
        {info.title || preview.title}
      </div>
      <div className="flex w-full min-w-0 items-center gap-2 text-xs leading-4">
        <span className="shrink-0 font-medium text-[color:var(--status-added)]">
          +{info.additions.toLocaleString()}
        </span>
        <span className="shrink-0 font-medium text-[color:var(--status-deleted)]">
          −{info.deletions.toLocaleString()}
        </span>
        <span className="min-w-0 truncate text-muted-foreground">
          {info.changedFiles.toLocaleString()}{" "}
          {info.changedFiles === 1 ? "file" : "files"} changed
        </span>
        <ExternalLink
          aria-hidden="true"
          className="ml-auto h-3.5 w-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover/attachment:opacity-100 group-focus-within/attachment:opacity-100"
        />
      </div>
      <AttachmentTrigger asChild>
        <a
          aria-label={`Open GitHub PR ${ref.owner}/${ref.repo} #${ref.number}: ${info.title || preview.title}`}
          href={preview.href}
          rel="noreferrer"
          target="_blank"
        >
          <span className="sr-only">
            Open GitHub PR {ref.owner}/{ref.repo} #{ref.number}
          </span>
        </a>
      </AttachmentTrigger>
    </Attachment>
  );
}

export function LinkPreviewAttachment({
  className,
  preview,
}: {
  className?: string;
  preview: SupportedLinkPreview;
}) {
  return (
    <Attachment
      className={cn(
        "w-80 max-w-full shrink-0 no-underline shadow-none",
        className,
      )}
      data-link-preview={preview.kind}
    >
      <AttachmentMedia className="link-preview-media">
        <LinkPreviewLogo preview={preview} />
      </AttachmentMedia>
      <AttachmentContent>
        <div className="truncate text-xs font-medium leading-4 text-muted-foreground">
          {preview.provider}
          <span aria-hidden="true"> · </span>
          {preview.typeLabel}
        </div>
        <AttachmentTitle>{preview.title}</AttachmentTitle>
      </AttachmentContent>
      <AttachmentActions>
        <ExternalLink
          aria-hidden="true"
          className="h-4 w-4 text-muted-foreground opacity-0 transition-opacity group-hover/attachment:opacity-100 group-focus-within/attachment:opacity-100"
        />
      </AttachmentActions>
      <AttachmentTrigger asChild>
        <a
          aria-label={`Open ${preview.provider} ${preview.typeLabel}: ${preview.title}`}
          href={preview.href}
          rel="noreferrer"
          target="_blank"
        >
          <span className="sr-only">
            Open {preview.provider} {preview.typeLabel}: {preview.title}
          </span>
        </a>
      </AttachmentTrigger>
    </Attachment>
  );
}
