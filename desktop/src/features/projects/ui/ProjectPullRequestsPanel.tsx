import {
  Check,
  FileCode2,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  MessageSquare,
  UserPlus,
  X,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type Project,
  type ProjectPullRequest,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import { projectPullRequestCommentTimelineKind } from "@/features/projects/projectPullRequests.mjs";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Markdown } from "@/shared/ui/markdown";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import { CopyCommitHashButton } from "./ProjectCommitCopyButton";
import type { OpenMergeRecoveryTerminal } from "./MergePullRequestButton";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import {
  ProfileAuthorName,
  ProfileIdentityButton,
} from "./ProjectProfileIdentity";
import { PullRequestReviewersRow } from "./PullRequestReviewersRow";
import { PullRequestReviewCard } from "./PullRequestReviewCard";

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

function relativeCreatedAt(createdAt: number) {
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
  avatarSize = "md",
  profiles,
  pubkey,
  role,
}: {
  avatarSize?: "xs" | "sm" | "md";
  profiles?: UserProfileLookup;
  pubkey: string;
  role?: React.ReactNode;
}) {
  const profile = profileForPubkey(pubkey, profiles);
  return (
    <ProfileIdentityButton
      align="center"
      avatarSize={avatarSize}
      avatarUrl={profile?.avatarUrl ?? null}
      isAgent={profile?.isAgent === true}
      label={labelForPubkey(pubkey, profiles)}
      pubkey={pubkey}
      role={role}
    />
  );
}

/** Commit hash chip that jumps to the commit detail when a handler is given. */
function CommitHashChip({
  hash,
  onOpenCommit,
}: {
  hash: string;
  onOpenCommit?: (commitHash: string) => void;
}) {
  const short = hash.slice(0, 7);
  if (!onOpenCommit) {
    return (
      <code className="shrink-0 rounded-md bg-background/55 px-2 py-1 text-xs text-muted-foreground">
        {short}
      </code>
    );
  }
  return (
    <button
      aria-label={`View commit ${short}`}
      className="shrink-0 rounded-md bg-background/55 px-2 py-1 font-mono text-xs text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      onClick={() => onOpenCommit(hash)}
      type="button"
    >
      {short}
    </button>
  );
}

function PullRequestCommitRow({
  author,
  branch,
  createdAt,
  hash,
  message,
  onOpenCommit,
  profiles,
}: {
  author: string;
  branch: string | null;
  createdAt: number;
  hash: string | null;
  message: string;
  onOpenCommit?: (commitHash: string) => void;
  profiles?: UserProfileLookup;
}) {
  const authorProfile = profileForPubkey(author, profiles);
  const authorLabel = labelForPubkey(author, profiles);
  const openCommit =
    hash && onOpenCommit ? () => onOpenCommit(hash) : undefined;

  return (
    <ProjectFeedRow
      meta={
        <>
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={authorProfile?.avatarUrl ?? null}
            isAgent={authorProfile?.isAgent === true}
            label={authorLabel}
            pubkey={author}
            showLabel={false}
          />
          <span className="truncate">
            <ProfileAuthorName pubkey={author}>{authorLabel}</ProfileAuthorName>{" "}
            authored {relativeTime(createdAt)}
          </span>
          {branch ? (
            <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
              <GitBranch className="h-3 w-3 shrink-0" />
              <span className="truncate">{branch}</span>
            </span>
          ) : null}
        </>
      }
      onOpen={openCommit}
      testId="project-pull-request-commit-row"
      title={message}
      trailing={
        hash ? (
          <ProjectFeedRowCluster>
            <ProjectFeedRowMonoCell
              label={hash.slice(0, 7)}
              onClick={openCommit}
              title={`View commit ${hash.slice(0, 7)}`}
            />
            <CopyCommitHashButton hash={hash} />
          </ProjectFeedRowCluster>
        ) : undefined
      }
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
  const authorProfile = profileForPubkey(pullRequest.author, profiles);
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const StatusIcon =
    pullRequest.status === "Closed" || pullRequest.status === "Draft"
      ? X
      : Check;
  const statusClassName = pullRequestStatusClassName(pullRequest.status);

  return (
    <ProjectFeedRow
      meta={
        <>
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={authorProfile?.avatarUrl ?? null}
            isAgent={authorProfile?.isAgent === true}
            label={authorLabel}
            pubkey={pullRequest.author}
            showLabel={false}
          />
          <span className="truncate">
            <ProfileAuthorName pubkey={pullRequest.author}>
              {authorLabel}
            </ProfileAuthorName>{" "}
            created this pull request {relativeCreatedAt(pullRequest.createdAt)}
          </span>
          {pullRequest.branchName ? (
            <span className="inline-flex min-w-0 items-center gap-1 rounded-full border border-border/60 px-1.5 py-0.5 font-mono text-2xs">
              <GitBranch className="h-3 w-3 shrink-0" />
              <span className="truncate">{pullRequest.branchName}</span>
            </span>
          ) : null}
          <span
            className={`rounded-full border border-border/60 px-1.5 py-0.5 text-2xs font-medium ${statusClassName}`}
          >
            {pullRequest.status}
          </span>
        </>
      }
      onOpen={onOpen}
      statusIcon={
        <StatusIcon className={`h-3.5 w-3.5 shrink-0 ${statusClassName}`} />
      }
      testId="project-pull-request-row"
      title={pullRequest.title}
      trailing={
        <>
          {pullRequest.comments.length > 0 ? (
            <button
              aria-label={`View ${pullRequest.comments.length} comments`}
              className="flex items-center gap-1 rounded-md text-xs text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onOpen}
              type="button"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {pullRequest.comments.length}
            </button>
          ) : null}
          <ProjectFeedRowCluster>
            <ProjectFeedRowMonoCell
              label={`#${pullRequest.id.slice(0, 8)}`}
              onClick={onOpen}
              title="View pull request"
            />
          </ProjectFeedRowCluster>
        </>
      }
    />
  );
}

export type PullRequestPanelMode = "conversation" | "commits" | "checks";

/** GitHub-style PR title line, rendered as the top section of the PR detail
 * card. Status, branches, and dates live in the right-hand meta rail. */
export function PullRequestDetailHeader({
  profiles,
  pullRequest,
}: {
  profiles?: UserProfileLookup;
  pullRequest: ProjectPullRequest;
}) {
  const authorLabel = labelForPubkey(pullRequest.author, profiles);

  return (
    <header className="min-w-0 space-y-1.5 p-4 pb-2">
      <h3 className="line-clamp-2 min-w-0 text-base font-semibold text-foreground">
        {pullRequest.title}{" "}
        <span className="font-normal text-muted-foreground">
          #{pullRequest.id.slice(0, 8)}
        </span>
      </h3>
      <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
        <GitPullRequest className="h-3.5 w-3.5" />
        Created {compactDate(pullRequest.createdAt)} by {authorLabel}
      </p>
    </header>
  );
}

/** Right-hand meta column for the PR detail view. */
export function PullRequestMetaRail({
  profiles,
  project,
  pullRequest,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const identityQuery = useIdentityQuery();
  const authorProfile = profileForPubkey(pullRequest.author, profiles);
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const targetBranch =
    pullRequest.targetBranch || project.defaultBranch || "default branch";
  const sourceBranch = pullRequest.branchName || "unknown branch";
  const commitCount = Math.max(1, pullRequest.updateCount + 1);
  const viewerPubkey = identityQuery.data?.pubkey;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(pullRequest.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  const canRequestReview =
    Boolean(viewer) && (isAuthor || isOwner || isManagedAgentOwner);

  return (
    <aside className="min-w-0 space-y-6 border-t border-border/60 p-4 xl:border-l xl:border-t-0">
      <OverviewRailSection title="Status">
        <span
          className={`inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium text-white ${pullRequestStatusBadgeClassName(pullRequest.status)}`}
        >
          {pullRequest.status === "Merged" ? (
            <GitMerge className="h-3.5 w-3.5" />
          ) : (
            <GitPullRequest className="h-3.5 w-3.5" />
          )}
          {pullRequest.status}
        </span>
      </OverviewRailSection>
      {pullRequest.reviewers.length > 0 || canRequestReview ? (
        <OverviewRailSection title="Reviewers">
          <PullRequestReviewersRow
            canRequest={canRequestReview}
            profiles={profiles}
            project={project}
            pullRequest={pullRequest}
            signAsManagedOwner={isManagedAgentOwner && !isOwner}
          />
        </OverviewRailSection>
      ) : null}
      <OverviewRailSection title="Author">
        <ProfileIdentityButton
          align="center"
          avatarSize="xs"
          avatarUrl={authorProfile?.avatarUrl ?? null}
          isAgent={authorProfile?.isAgent === true}
          label={authorLabel}
          pubkey={pullRequest.author}
        />
      </OverviewRailSection>
      <OverviewRailSection title="Branches">
        <div className="space-y-1.5 text-xs text-muted-foreground">
          <p>Merges {pluralize(commitCount, "commit")}</p>
          <p className="flex min-w-0 flex-wrap items-center gap-1.5">
            <code className="rounded-sm bg-muted px-1.5 py-0.5 text-2xs text-foreground">
              {sourceBranch}
            </code>
            <span aria-hidden>→</span>
            <code className="rounded-sm bg-muted px-1.5 py-0.5 text-2xs text-foreground">
              {targetBranch}
            </code>
          </p>
        </div>
      </OverviewRailSection>
      <OverviewRailSection title="Activity">
        <dl className="space-y-1.5 text-xs text-muted-foreground">
          <div className="flex items-center justify-between gap-3">
            <dt>Created</dt>
            <dd className="font-medium text-foreground">
              {compactDate(pullRequest.createdAt)}
            </dd>
          </div>
          <div className="flex items-center justify-between gap-3">
            <dt>Updated</dt>
            <dd className="font-medium text-foreground">
              {compactDate(pullRequest.updatedAt)}
            </dd>
          </div>
        </dl>
      </OverviewRailSection>
    </aside>
  );
}

function PullRequestDetail({
  mode,
  onOpenCommit,
  onOpenTerminal,
  profiles,
  project,
  pullRequest,
}: {
  mode: PullRequestPanelMode;
  onOpenCommit?: (commitHash: string) => void;
  onOpenTerminal?: OpenMergeRecoveryTerminal;
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
    const commitCount = Math.max(1, pullRequest.updates.length + 1);
    return (
      <section>
        <header className="flex min-h-10 items-center gap-2 border-b border-border/50 bg-muted/20 px-4">
          <GitCommitHorizontal className="h-4 w-4 text-muted-foreground" />
          <h4 className="text-sm font-medium text-foreground">Commits</h4>
          <span className="rounded-full bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">
            {commitCount}
          </span>
        </header>
        <div className="divide-y divide-border/50">
          <PullRequestCommitRow
            author={pullRequest.author}
            branch={pullRequest.branchName}
            createdAt={pullRequest.createdAt}
            hash={pullRequest.commit}
            message={pullRequest.title}
            onOpenCommit={onOpenCommit}
            profiles={profiles}
          />
          {pullRequest.updates.map((update) => (
            <PullRequestCommitRow
              author={update.author}
              branch={pullRequest.branchName}
              createdAt={update.createdAt}
              hash={update.commit}
              key={update.id}
              message={update.content.trim() || "Updated pull request branch"}
              onOpenCommit={onOpenCommit}
              profiles={profiles}
            />
          ))}
        </div>
      </section>
    );
  }

  if (mode === "checks") {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        No checks have been reported for this pull request yet.
      </p>
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
          <h4 className="text-sm font-semibold text-foreground">Updates</h4>
          {pullRequest.updates.map((update) => (
            <article className="space-y-1" key={update.id}>
              <div className="flex min-w-0 items-center justify-between gap-3">
                <AuthorIdentity
                  profiles={profiles}
                  pubkey={update.author}
                  role={compactDate(update.createdAt)}
                />
                {update.commit ? (
                  <CommitHashChip
                    hash={update.commit}
                    onOpenCommit={onOpenCommit}
                  />
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
        {pullRequest.comments.length > 0 ? (
          <div className="-mt-4">
            {pullRequest.comments.map((item) => {
              // Review decisions and requests render as compact timeline
              // rows (GitHub-style) rather than full comment cards.
              const timelineKind = projectPullRequestCommentTimelineKind(item);
              if (timelineKind) {
                const isHistoricalDecision =
                  item.reviewDecisionStatus === "historical";
                return (
                  <div
                    className="-mx-4 flex min-h-10 min-w-0 items-center gap-2 border-b border-border/50 px-4 text-sm text-muted-foreground"
                    key={item.id}
                  >
                    {timelineKind === "approved" ? (
                      <Check
                        className={`h-3.5 w-3.5 shrink-0 ${
                          isHistoricalDecision
                            ? "text-muted-foreground"
                            : "text-green-600 dark:text-green-500"
                        }`}
                      />
                    ) : timelineKind === "changes-requested" ? (
                      <X
                        className={`h-3.5 w-3.5 shrink-0 ${
                          isHistoricalDecision
                            ? "text-muted-foreground"
                            : "text-destructive"
                        }`}
                      />
                    ) : (
                      <UserPlus className="h-3.5 w-3.5 shrink-0" />
                    )}
                    <span className="flex min-w-0 flex-1 items-center gap-1.5 overflow-hidden">
                      <span className="shrink-0 font-medium text-foreground">
                        {labelForPubkey(item.author, profiles)}
                      </span>
                      <span className="min-w-0 truncate">
                        {timelineKind === "approved"
                          ? isHistoricalDecision
                            ? "approved an earlier commit"
                            : "approved these changes"
                          : timelineKind === "changes-requested"
                            ? isHistoricalDecision
                              ? "requested changes on an earlier commit"
                              : "requested changes"
                            : item.content.trim() || "requested a review"}
                      </span>
                    </span>
                    <span className="w-20 shrink-0 text-right text-xs text-muted-foreground/70">
                      {compactDate(item.createdAt)}
                    </span>
                  </div>
                );
              }
              return (
                <article className="py-3" key={item.id}>
                  {item.anchor ? (
                    <div className="mb-2 flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                      <FileCode2 className="h-3.5 w-3.5 shrink-0" />
                      <span className="truncate">{item.anchor.path}</span>
                      <span className="shrink-0 font-mono">
                        {item.anchor.side === "new" ? "+" : "-"}
                        {item.anchor.line}
                      </span>
                      {item.inlineCommentStatus === "outdated" ? (
                        <span className="shrink-0 rounded-full bg-muted px-1.5 py-0.5 text-2xs">
                          Outdated
                        </span>
                      ) : null}
                    </div>
                  ) : null}
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
              );
            })}
          </div>
        ) : null}
        <PullRequestReviewCard
          onOpenTerminal={onOpenTerminal}
          project={project}
          pullRequest={pullRequest}
        />
        <h4 className="flex items-center gap-1.5 text-sm font-semibold text-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Add Your Comment
        </h4>
        <ForumComposer
          className="border border-border/60 bg-background/45"
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
  onOpenCommit,
  onOpenTerminal,
  onSelectedPullRequestIdChange,
  profiles,
  project,
  pullRequests,
  selectedPullRequestId,
}: {
  error: unknown;
  isLoading: boolean;
  mode?: PullRequestPanelMode;
  onOpenCommit?: (commitHash: string) => void;
  onOpenTerminal?: OpenMergeRecoveryTerminal;
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
        onOpenCommit={onOpenCommit}
        onOpenTerminal={onOpenTerminal}
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
