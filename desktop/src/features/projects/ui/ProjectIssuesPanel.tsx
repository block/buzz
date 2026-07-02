import { CircleCheck, CircleDot, CircleX, MessageSquare } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type Project,
  type ProjectIssue,
  useCreateProjectIssueCommentMutation,
  useProjectIssuesQuery,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function relativeOpenedAt(createdAt: number) {
  const elapsedSeconds = Math.max(
    1,
    Math.floor(Date.now() / 1_000 - createdAt),
  );
  const units = [
    { label: "year", seconds: 365 * 24 * 60 * 60 },
    { label: "month", seconds: 30 * 24 * 60 * 60 },
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "minute", seconds: 60 },
  ];
  const unit =
    units.find((item) => elapsedSeconds >= item.seconds) ??
    units[units.length - 1];
  const value = Math.max(1, Math.floor(elapsedSeconds / unit.seconds));
  return `${value} ${unit.label}${value === 1 ? "" : "s"} ago`;
}

export function issueStatusClassName(status: ProjectIssue["status"]) {
  if (status === "Done") return "text-purple-400";
  if (status === "Closed") return "text-destructive";
  return "text-green-500";
}

function issueStatusVisual(status: ProjectIssue["status"]) {
  if (status === "Done") {
    return { className: "text-purple-400", icon: CircleCheck };
  }
  if (status === "Closed") {
    return { className: "text-destructive", icon: CircleX };
  }
  return { className: "text-green-500", icon: CircleDot };
}

function issueMembers(
  project: Project,
  issue: ProjectIssue,
  profiles?: UserProfileLookup,
): ChannelMember[] {
  return [
    ...new Set([
      project.owner,
      issue.author,
      ...project.contributors,
      ...issue.recipients,
    ]),
  ].map((pubkey) => {
    const profile = profiles?.[normalizePubkey(pubkey)];
    return {
      pubkey,
      role: "member" as const,
      isAgent: profile?.isAgent === true,
      joinedAt: new Date(0).toISOString(),
      displayName:
        profile?.displayName?.trim() || profile?.nip05Handle?.trim() || null,
    };
  });
}

function AuthorIdentity({
  profiles,
  pubkey,
  role,
}: {
  profiles?: UserProfileLookup;
  pubkey: string;
  role?: React.ReactNode;
}) {
  const profile = profiles?.[normalizePubkey(pubkey)];
  return (
    <ProfileIdentityButton
      align="center"
      avatarSize="xs"
      avatarUrl={profile?.avatarUrl ?? null}
      isAgent={profile?.isAgent === true}
      label={resolveUserLabel({ profiles, pubkey })}
      pubkey={pubkey}
      role={role}
    />
  );
}

function IssueRow({
  issue,
  onOpen,
  profiles,
}: {
  issue: ProjectIssue;
  onOpen: () => void;
  profiles?: UserProfileLookup;
}) {
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const status = issueStatusVisual(issue.status);

  return (
    <button
      className="flex w-full min-w-0 items-start gap-3 p-3 text-left transition-colors hover:bg-muted/30 focus-visible:bg-muted/30 focus-visible:outline-hidden"
      onClick={onOpen}
      type="button"
    >
      <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <CircleDot className="h-5 w-5" />
      </div>
      <div className="min-w-0 flex-1 space-y-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="truncate text-sm font-semibold leading-5 text-foreground">
            {issue.title}
          </p>
          <status.icon className={`h-3.5 w-3.5 shrink-0 ${status.className}`} />
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
          <span>#{issue.id.slice(0, 8)}</span>
          <span>opened {relativeOpenedAt(issue.createdAt)}</span>
          <span>by {authorLabel}</span>
          <span>·</span>
          <span>{issue.status}</span>
          {issue.labels.map((label) => (
            <span
              className="rounded-full border border-border/50 px-1.5 py-0.5 text-2xs"
              key={label}
            >
              {label}
            </span>
          ))}
        </div>
      </div>
      {issue.comments.length > 0 ? (
        <span className="mt-1 flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          {issue.comments.length}
        </span>
      ) : null}
    </button>
  );
}

function IssueDetail({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const commentMutation = useCreateProjectIssueCommentMutation(project);
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const members = React.useMemo(
    () => issueMembers(project, issue, profiles),
    [issue, profiles, project],
  );
  const handleCommentSubmit = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      try {
        await commentMutation.mutateAsync({
          content,
          issue,
          mediaTags,
          mentionPubkeys,
        });
        toast.success("Comment posted.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to post comment.",
        );
        throw error;
      }
    },
    [commentMutation, issue],
  );

  return (
    <div className="divide-y divide-border/50">
      <header className="space-y-3 p-4">
        <div className="min-w-0 space-y-2">
          <div className="min-w-0">
            <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <CircleDot className="h-3.5 w-3.5" />
              Issue from {authorLabel}
              <span
                className={`rounded-full border border-border/50 px-1.5 py-0.5 text-2xs ${issueStatusClassName(issue.status)}`}
              >
                {issue.status}
              </span>
            </p>
            <h3 className="mt-1 line-clamp-2 text-base font-semibold text-foreground">
              {issue.title}
            </h3>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
            <span>#{issue.id.slice(0, 8)}</span>
            <span>·</span>
            <span>opened {compactDate(issue.createdAt)}</span>
            <span>·</span>
            <span>updated {compactDate(issue.updatedAt)}</span>
            {issue.labels.map((label) => (
              <span
                className="rounded-full border border-border/50 px-1.5 py-0.5 text-2xs"
                key={label}
              >
                {label}
              </span>
            ))}
          </div>
        </div>
        {issue.content ? (
          <div className="rounded-lg border border-border/50 bg-background/45 p-3">
            <Markdown
              className="text-sm"
              content={issue.content}
              interactive={false}
            />
          </div>
        ) : null}
      </header>

      <section className="space-y-3 p-4">
        <h4 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Discussion
        </h4>
        {issue.comments.length > 0 ? (
          <div className="space-y-3">
            {issue.comments.map((item) => (
              <article
                className="rounded-lg border border-border/50 bg-background/45 p-3"
                key={item.id}
              >
                <div className="mb-2">
                  <AuthorIdentity
                    profiles={profiles}
                    pubkey={item.author}
                    role={compactDate(item.createdAt)}
                  />
                </div>
                <Markdown
                  className="text-sm"
                  content={item.content}
                  interactive={false}
                />
              </article>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">No comments yet.</p>
        )}
        <ForumComposer
          className="rounded-lg border border-border/50 bg-background/45"
          disabled={commentMutation.isPending}
          isSending={commentMutation.isPending}
          members={members}
          onSubmit={handleCommentSubmit}
          placeholder="Add a comment…"
          profiles={profiles}
        />
      </section>
    </div>
  );
}

export function ProjectIssuesPanel({
  onSelectedIssueIdChange,
  profiles,
  project,
  selectedIssueId,
}: {
  onSelectedIssueIdChange: (id: string | null) => void;
  profiles?: UserProfileLookup;
  project: Project;
  selectedIssueId: string | null;
}) {
  const issuesQuery = useProjectIssuesQuery(project);
  const issues = issuesQuery.data ?? [];
  const selectedIssue =
    issues.find((issue) => issue.id === selectedIssueId) ?? null;

  if (issuesQuery.isLoading) {
    return <p className="p-4 text-sm text-muted-foreground">Loading issues…</p>;
  }

  if (issues.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {issuesQuery.error
          ? "Could not load issues for this repository."
          : "No issues yet."}
      </p>
    );
  }

  if (selectedIssue) {
    return (
      <IssueDetail
        issue={selectedIssue}
        profiles={profiles}
        project={project}
      />
    );
  }

  return (
    <div className="divide-y divide-border/50">
      {issues.map((issue) => (
        <IssueRow
          issue={issue}
          key={issue.id}
          onOpen={() => onSelectedIssueIdChange(issue.id)}
          profiles={profiles}
        />
      ))}
    </div>
  );
}
