import {
  Check,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  X,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type Project,
  type ProjectPullRequest,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function profileForPubkey(pubkey: string, profiles?: UserProfileLookup) {
  return profiles?.[normalizePubkey(pubkey)] ?? null;
}

function labelForPubkey(pubkey: string, profiles?: UserProfileLookup) {
  const profile = profileForPubkey(pubkey, profiles);
  return (
    profile?.displayName?.trim() ||
    profile?.nip05Handle?.trim() ||
    truncatePubkey(pubkey)
  );
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

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

function pullRequestStatusClassName(status: ProjectPullRequest["status"]) {
  if (status === "Closed") return "text-destructive";
  if (status === "Draft") return "text-muted-foreground";
  if (status === "Merged") return "text-purple-400";
  return "text-green-500";
}

function pullRequestStatusBadgeClassName(status: ProjectPullRequest["status"]) {
  if (status === "Closed") return "bg-destructive";
  if (status === "Draft") return "bg-muted-foreground/80";
  if (status === "Merged") return "bg-purple-600";
  return "bg-green-600";
}

function pullRequestMembers(
  project: Project,
  pullRequest: ProjectPullRequest,
  profiles?: UserProfileLookup,
): ChannelMember[] {
  return [
    ...new Set([
      project.owner,
      pullRequest.author,
      ...project.contributors,
      ...pullRequest.recipients,
    ]),
  ].map((pubkey) => {
    const profile = profileForPubkey(pubkey, profiles);
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
  const profile = profileForPubkey(pubkey, profiles);
  return (
    <ProfileIdentityButton
      align="center"
      avatarSize="xs"
      avatarUrl={profile?.avatarUrl ?? null}
      isAgent={profile?.isAgent === true}
      label={labelForPubkey(pubkey, profiles)}
      pubkey={pubkey}
      role={role}
    />
  );
}

function PullRequestRow({
  onOpen,
  profiles,
  pullRequest,
}: {
  onOpen: () => void;
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
}) {
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const StatusIcon =
    pullRequest.status === "Closed" || pullRequest.status === "Draft"
      ? X
      : Check;
  const statusClassName = pullRequestStatusClassName(pullRequest.status);

  return (
    <button
      className="flex w-full min-w-0 items-start gap-3 p-3 text-left transition-colors hover:bg-muted/30 focus-visible:bg-muted/30 focus-visible:outline-hidden"
      onClick={onOpen}
      type="button"
    >
      <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
        <GitPullRequest className="h-5 w-5" />
      </div>
      <div className="min-w-0 flex-1 space-y-0.5">
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="truncate text-sm font-semibold leading-5 text-foreground">
            {pullRequest.title}
          </p>
          <StatusIcon className={`h-3.5 w-3.5 shrink-0 ${statusClassName}`} />
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
          <span>#{pullRequest.id.slice(0, 8)}</span>
          <span>opened {relativeOpenedAt(pullRequest.createdAt)}</span>
          <span>by {authorLabel}</span>
          <span className="rounded-full border border-border/50 px-1.5 py-0.5 text-2xs">
            Member
          </span>
          <span>·</span>
          <span>{pullRequest.status}</span>
        </div>
      </div>
      {pullRequest.comments.length > 0 ? (
        <span className="mt-1 flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          {pullRequest.comments.length}
        </span>
      ) : null}
    </button>
  );
}

export type PullRequestPanelMode = "conversation" | "commits" | "checks";

/** GitHub-style PR title + status line, rendered above the PR tab row. */
export function PullRequestDetailHeader({
  profiles,
  project,
  pullRequest,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const targetBranch = project.defaultBranch || "default branch";
  const sourceBranch = pullRequest.branchName || "unknown branch";
  const commitCount = Math.max(1, pullRequest.updateCount + 1);

  return (
    <div className="min-w-0 space-y-2.5">
      <h3 className="min-w-0 text-xl font-semibold leading-snug text-foreground">
        {pullRequest.title}{" "}
        <span className="font-normal text-muted-foreground">
          #{pullRequest.id.slice(0, 8)}
        </span>
      </h3>
      <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1.5 text-xs leading-4 text-muted-foreground">
        <span
          className={`mr-1 inline-flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium text-white ${pullRequestStatusBadgeClassName(pullRequest.status)}`}
        >
          {pullRequest.status === "Merged" ? (
            <GitMerge className="h-3.5 w-3.5" />
          ) : (
            <GitPullRequest className="h-3.5 w-3.5" />
          )}
          {pullRequest.status}
        </span>
        <span className="font-medium text-foreground">{authorLabel}</span>
        <span>wants to merge {pluralize(commitCount, "commit")} into</span>
        <code className="rounded-md bg-muted px-1.5 py-0.5 text-2xs text-foreground">
          {targetBranch}
        </code>
        <span>from</span>
        <code className="rounded-md bg-muted px-1.5 py-0.5 text-2xs text-foreground">
          {sourceBranch}
        </code>
        <span>·</span>
        <span>opened {compactDate(pullRequest.createdAt)}</span>
        <span>·</span>
        <span>updated {compactDate(pullRequest.updatedAt)}</span>
      </div>
    </div>
  );
}

function PullRequestDetail({
  mode,
  profiles,
  project,
  pullRequest,
}: {
  mode: PullRequestPanelMode;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const commentMutation = useCreateProjectPullRequestCommentMutation(project);
  const members = React.useMemo(
    () => pullRequestMembers(project, pullRequest, profiles),
    [profiles, project, pullRequest],
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
          mediaTags,
          mentionPubkeys,
          pullRequest,
        });
        toast.success("Comment posted.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to post comment.",
        );
        throw error;
      }
    },
    [commentMutation, pullRequest],
  );

  if (mode === "commits") {
    return (
      <div className="divide-y divide-border/50">
        <section className="space-y-3 p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Commits
          </h4>
          <article className="space-y-1">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <AuthorIdentity
                profiles={profiles}
                pubkey={pullRequest.author}
                role={compactDate(pullRequest.createdAt)}
              />
              {pullRequest.commit ? (
                <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
                  {pullRequest.commit.slice(0, 7)}
                </code>
              ) : null}
            </div>
            <p className="text-sm text-muted-foreground">{pullRequest.title}</p>
          </article>
          {pullRequest.updates.map((update) => (
            <article className="space-y-1" key={update.id}>
              <div className="flex min-w-0 items-center justify-between gap-3">
                <AuthorIdentity
                  profiles={profiles}
                  pubkey={update.author}
                  role={compactDate(update.createdAt)}
                />
                {update.commit ? (
                  <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
                    {update.commit.slice(0, 7)}
                  </code>
                ) : null}
              </div>
              {update.content ? (
                <p className="text-sm text-muted-foreground">
                  {update.content}
                </p>
              ) : null}
            </article>
          ))}
        </section>
      </div>
    );
  }

  if (mode === "checks") {
    return (
      <div className="p-4">
        <div className="rounded-lg border border-border/50 bg-background/45 p-4 text-sm text-muted-foreground">
          No checks have been reported for this pull request yet.
        </div>
      </div>
    );
  }

  return (
    <div className="divide-y divide-border/50">
      {pullRequest.content ? (
        <header className="p-4">
          <Markdown
            className="text-sm"
            content={pullRequest.content}
            interactive={false}
          />
        </header>
      ) : null}

      {pullRequest.updates.length > 0 ? (
        <section className="space-y-3 p-4">
          <h4 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Updates
          </h4>
          {pullRequest.updates.map((update) => (
            <article className="space-y-1" key={update.id}>
              <div className="flex min-w-0 items-center justify-between gap-3">
                <AuthorIdentity
                  profiles={profiles}
                  pubkey={update.author}
                  role={compactDate(update.createdAt)}
                />
                {update.commit ? (
                  <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
                    {update.commit.slice(0, 7)}
                  </code>
                ) : null}
              </div>
              {update.content ? (
                <p className="text-sm text-muted-foreground">
                  {update.content}
                </p>
              ) : null}
            </article>
          ))}
        </section>
      ) : null}

      <section className="space-y-3 p-4">
        <h4 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Discussion
        </h4>
        {pullRequest.comments.length > 0 ? (
          <div className="space-y-3">
            {pullRequest.comments.map((item) => (
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

export function PullRequestsPanel({
  error,
  isLoading,
  mode = "conversation",
  onSelectedPullRequestIdChange,
  profiles,
  project,
  pullRequests,
  selectedPullRequestId,
}: {
  error: unknown;
  isLoading: boolean;
  mode?: PullRequestPanelMode;
  onSelectedPullRequestIdChange: (id: string | null) => void;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequests: ProjectPullRequest[];
  selectedPullRequestId: string | null;
}) {
  const selectedPullRequest =
    pullRequests.find((item) => item.id === selectedPullRequestId) ?? null;

  React.useEffect(() => {
    if (
      selectedPullRequestId &&
      !pullRequests.some((item) => item.id === selectedPullRequestId)
    ) {
      onSelectedPullRequestIdChange(null);
    }
  }, [onSelectedPullRequestIdChange, pullRequests, selectedPullRequestId]);

  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        Loading pull requests…
      </p>
    );
  }

  if (pullRequests.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load pull requests for this repository."
          : "No pull requests yet."}
      </p>
    );
  }

  if (selectedPullRequest) {
    return (
      <PullRequestDetail
        mode={mode}
        profiles={profiles}
        project={project}
        pullRequest={selectedPullRequest}
      />
    );
  }

  return (
    <div className="divide-y divide-border/50">
      {pullRequests.map((pullRequest) => (
        <PullRequestRow
          key={pullRequest.id}
          onOpen={() => onSelectedPullRequestIdChange(pullRequest.id)}
          profiles={profiles}
          pullRequest={pullRequest}
        />
      ))}
    </div>
  );
}
