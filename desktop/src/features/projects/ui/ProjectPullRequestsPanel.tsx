import {
  Check,
  ChevronDown,
  ChevronUp,
  FileCode2,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  GitPullRequest,
  History,
  MessageSquare,
  TriangleAlert,
  UserPlus,
  X,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type Project,
  type ProjectPullRequest,
  type ProjectPullRequestCommentAnchor,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import { projectPullRequestCommentTimelineKind } from "@/features/projects/projectPullRequests.mjs";
import {
  formatExactTimestamp,
  relativeTime,
} from "@/features/projects/lib/projectsViewHelpers";
import { canReviewProjectPullRequest } from "@/features/projects/pullRequestReviews";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
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
import { ProjectRichContent } from "./ProjectRichContent";
import { PullRequestReviewersRow } from "./PullRequestReviewersRow";
import { PullRequestReviewCard } from "./PullRequestReviewCard";

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
  showLabel = true,
}: {
  avatarSize?: "xs" | "sm" | "md";
  profiles?: UserProfileLookup;
  pubkey: string;
  role?: React.ReactNode;
  showLabel?: boolean;
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
      showLabel={showLabel}
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
            authored{" "}
            <span title={formatExactTimestamp(createdAt)}>
              {relativeTime(createdAt)}
            </span>
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
            created this pull request{" "}
            <span title={formatExactTimestamp(pullRequest.createdAt)}>
              {relativeTime(pullRequest.createdAt)}
            </span>
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
  const sourceChannelId = pullRequest.channelId;
  const { goChannel } = useAppNavigation();
  const channelsQuery = useChannelsQuery({
    enabled: Boolean(sourceChannelId),
  });
  const sourceChannel = channelsQuery.data?.find(
    (channel) => channel.id === sourceChannelId,
  );

  return (
    <header className="min-w-0 space-y-1 p-4 pb-4">
      <h3 className="line-clamp-2 min-w-0 text-xl font-semibold text-foreground">
        {pullRequest.title}{" "}
        <span className="font-normal text-muted-foreground">
          #{pullRequest.id.slice(0, 8)}
        </span>
      </h3>
      <p className="flex items-center gap-1 text-xs font-medium text-muted-foreground">
        <GitPullRequest className="h-3.5 w-3.5" />
        <span className="flex min-w-0 items-center gap-1">
          <AuthorIdentity
            avatarSize="xs"
            profiles={profiles}
            pubkey={pullRequest.author}
            showLabel={false}
          />
          <ProfileAuthorName pubkey={pullRequest.author}>
            {authorLabel}
          </ProfileAuthorName>
        </span>
        <span title={formatExactTimestamp(pullRequest.createdAt)}>
          created {relativeTime(pullRequest.createdAt)}
        </span>
        {sourceChannelId ? (
          <span
            className="inline-flex min-w-0 items-center gap-1"
            title="Source channel is claimed by the pull request author and is not relay-verified."
          >
            <span>linked from</span>
            {sourceChannel ? (
              <button
                aria-label={`Open author-claimed source channel #${sourceChannel.name}`}
                className="truncate font-medium text-foreground underline-offset-2 hover:underline"
                onClick={() => void goChannel(sourceChannel.id)}
                type="button"
              >
                #{sourceChannel.name}
              </button>
            ) : (
              <span>an unavailable channel</span>
            )}
            <span>(author-claimed)</span>
          </span>
        ) : null}
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
            <dd
              className="font-medium text-foreground"
              title={formatExactTimestamp(pullRequest.createdAt)}
            >
              {relativeTime(pullRequest.createdAt)}
            </dd>
          </div>
          <div className="flex items-center justify-between gap-3">
            <dt>Updated</dt>
            <dd
              className="font-medium text-foreground"
              title={formatExactTimestamp(pullRequest.updatedAt)}
            >
              {relativeTime(pullRequest.updatedAt)}
            </dd>
          </div>
        </dl>
      </OverviewRailSection>
    </aside>
  );
}

function PullRequestDetail({
  mode,
  onOpenInlineComment,
  onOpenCommit,
  onOpenTerminal,
  profiles,
  project,
  pullRequest,
}: {
  mode: PullRequestPanelMode;
  onOpenInlineComment?: (anchor: ProjectPullRequestCommentAnchor) => void;
  onOpenCommit?: (commitHash: string) => void;
  onOpenTerminal?: OpenMergeRecoveryTerminal;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const identityQuery = useIdentityQuery();
  const commentMutation = useCreateProjectPullRequestCommentMutation(project);
  const [
    expandedReviewHistoryPullRequestIds,
    setExpandedReviewHistoryPullRequestIds,
  ] = React.useState<Set<string>>(() => new Set());
  const [
    collapsedReviewHistoryPullRequestIds,
    setCollapsedReviewHistoryPullRequestIds,
  ] = React.useState<Set<string>>(() => new Set());
  const members = React.useMemo(
    () => pullRequestMembers(project, pullRequest, profiles),
    [profiles, project, pullRequest],
  );
  const submitComment = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      decision?: "request-changes",
    ) => {
      try {
        await commentMutation.mutateAsync({
          content,
          decision,
          mediaTags,
          mentionPubkeys,
          pullRequest,
        });
        toast.success(
          decision === "request-changes"
            ? "Changes requested."
            : "Comment posted.",
        );
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to post comment.",
        );
        throw error;
      }
    },
    [commentMutation, pullRequest],
  );
  const handleCommentSubmit = React.useCallback(
    (content: string, mentionPubkeys: string[], mediaTags?: string[][]) =>
      submitComment(content, mentionPubkeys, mediaTags),
    [submitComment],
  );
  const handleChangeRequestSubmit = React.useCallback(
    (content: string, mentionPubkeys: string[], mediaTags?: string[][]) =>
      submitComment(content, mentionPubkeys, mediaTags, "request-changes"),
    [submitComment],
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

  const reviewHistory = pullRequest.comments
    .map((item) => ({
      item,
      timelineKind: projectPullRequestCommentTimelineKind(item),
    }))
    .sort(
      (left, right) =>
        left.item.createdAt - right.item.createdAt ||
        left.item.id.localeCompare(right.item.id),
    );
  const reviewHistoryCollapsed = collapsedReviewHistoryPullRequestIds.has(
    pullRequest.id,
  );
  const reviewHistoryExpanded = expandedReviewHistoryPullRequestIds.has(
    pullRequest.id,
  );
  const earlierReviewHistoryCount = Math.max(0, reviewHistory.length - 3);
  const visibleReviewHistory =
    reviewHistoryExpanded || earlierReviewHistoryCount === 0
      ? reviewHistory
      : reviewHistory.slice(-3);
  const displayedReviewHistory = reviewHistoryCollapsed
    ? []
    : visibleReviewHistory;
  const canRequestChanges = canReviewProjectPullRequest(
    project,
    pullRequest,
    identityQuery.data?.pubkey,
  );

  return (
    <div>
      {pullRequest.content ? (
        <header className="p-4">
          <ProjectRichContent
            content={pullRequest.content}
            tags={pullRequest.tags}
          />
        </header>
      ) : null}

      {pullRequest.updates.length > 0 ? (
        <section className="space-y-3 border-border/50 border-t p-4">
          <h4 className="text-sm font-semibold text-foreground">Updates</h4>
          {pullRequest.updates.map((update) => (
            <article className="space-y-1" key={update.id}>
              <div className="flex min-w-0 items-center justify-between gap-3">
                <AuthorIdentity
                  profiles={profiles}
                  pubkey={update.author}
                  role={
                    <span title={formatExactTimestamp(update.createdAt)}>
                      {relativeTime(update.createdAt)}
                    </span>
                  }
                />
                {update.commit ? (
                  <CommitHashChip
                    hash={update.commit}
                    onOpenCommit={onOpenCommit}
                  />
                ) : null}
              </div>
              {update.content ? (
                <ProjectRichContent
                  className="text-sm text-muted-foreground"
                  content={update.content}
                  tags={update.tags}
                />
              ) : null}
            </article>
          ))}
        </section>
      ) : null}

      <section className="space-y-3 p-4">
        <div className="group/timeline -mx-4 overflow-hidden border-border/50 border-b">
          {reviewHistory.length > 0 ? (
            <button
              aria-expanded={!reviewHistoryCollapsed}
              className="flex min-h-10 w-full items-center gap-2 px-3 py-2.5 text-sm font-semibold text-muted-foreground transition-colors hover:text-foreground"
              data-testid="project-pull-request-review-history-toggle"
              onClick={() => {
                setCollapsedReviewHistoryPullRequestIds((current) => {
                  const next = new Set(current);
                  if (reviewHistoryCollapsed) {
                    next.delete(pullRequest.id);
                  } else {
                    next.add(pullRequest.id);
                  }
                  return next;
                });
              }}
              type="button"
            >
              <span className="relative flex w-5 shrink-0 justify-center self-stretch">
                {reviewHistoryCollapsed ? (
                  <span className="absolute top-2.5 -bottom-11 hidden w-px bg-border/80 group-has-[.pull-request-action-timeline]/timeline:block" />
                ) : (
                  <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
                )}
                <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-primary/10 text-primary ring-1 ring-primary/35">
                  <History className="h-3 w-3" />
                </span>
              </span>
              <span className="flex min-h-5 min-w-0 flex-1 items-center text-left">
                {reviewHistoryCollapsed
                  ? `Show ${reviewHistory.length} earlier ${
                      reviewHistory.length === 1 ? "activity" : "activities"
                    }`
                  : "Collapse review history"}
              </span>
              {reviewHistoryCollapsed ? (
                <ChevronDown className="mt-0.5 h-3.5 w-3.5" />
              ) : (
                <ChevronUp className="mt-0.5 h-3.5 w-3.5" />
              )}
            </button>
          ) : null}
          {!reviewHistoryCollapsed &&
          earlierReviewHistoryCount > 0 &&
          !reviewHistoryExpanded ? (
            <button
              className="flex min-h-10 w-full items-center gap-2 px-3 py-2.5 text-sm font-semibold text-muted-foreground transition-colors hover:text-foreground"
              data-testid="project-pull-request-earlier-activities"
              onClick={() => {
                setExpandedReviewHistoryPullRequestIds((current) => {
                  const next = new Set(current);
                  next.add(pullRequest.id);
                  return next;
                });
              }}
              type="button"
            >
              <span className="relative flex w-5 shrink-0 justify-center self-stretch">
                <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
                <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-background ring-1 ring-border/70">
                  <ChevronDown className="h-3 w-3" />
                </span>
              </span>
              <span className="min-w-0 flex-1 text-left">
                Show {earlierReviewHistoryCount} earlier{" "}
                {earlierReviewHistoryCount === 1 ? "activity" : "activities"}
              </span>
            </button>
          ) : null}
          {displayedReviewHistory.map(({ item, timelineKind }, index) => {
            const isHistoricalDecision =
              item.reviewDecisionStatus === "historical";
            const trimmedContent = item.content.trim();
            const activityContent =
              timelineKind === null
                ? trimmedContent
                : timelineKind === "changes-requested" &&
                    !/^requested changes\.?$/i.test(trimmedContent)
                  ? trimmedContent
                  : timelineKind === "approved" &&
                      !/^approved (these )?changes\.?$/i.test(trimmedContent)
                    ? trimmedContent
                    : null;
            return (
              <div
                className="flex min-h-10 min-w-0 items-start gap-2 px-3 py-2.5 text-sm text-muted-foreground"
                data-testid="project-pull-request-timeline-row"
                key={item.id}
              >
                <div className="relative flex w-5 shrink-0 justify-center self-stretch">
                  {index < displayedReviewHistory.length - 1 ? (
                    <span className="absolute top-2.5 -bottom-[1.875rem] w-px bg-border/80" />
                  ) : (
                    <span className="absolute top-2.5 -bottom-11 hidden w-px bg-border/80 group-has-[.pull-request-action-timeline]/timeline:block" />
                  )}
                  <span className="relative z-10 flex h-5 w-5 items-center justify-center rounded-full bg-background ring-1 ring-border/70">
                    {timelineKind === "approved" ? (
                      <Check
                        className={`h-3 w-3 ${
                          isHistoricalDecision
                            ? "text-muted-foreground"
                            : "text-green-600 dark:text-green-500"
                        }`}
                      />
                    ) : timelineKind === "changes-requested" ? (
                      <TriangleAlert
                        className={`h-3 w-3 ${
                          isHistoricalDecision
                            ? "text-muted-foreground"
                            : "text-amber-600 dark:text-amber-400"
                        }`}
                      />
                    ) : timelineKind === "review-request" ? (
                      <UserPlus className="h-3 w-3" />
                    ) : (
                      <MessageSquare className="h-3 w-3" />
                    )}
                  </span>
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex min-w-0 items-center">
                    <span className="min-w-0 truncate">
                      <ProfileAuthorName pubkey={item.author}>
                        {labelForPubkey(item.author, profiles)}
                      </ProfileAuthorName>
                      {timelineKind ? (
                        <>
                          {" "}
                          {timelineKind === "approved"
                            ? isHistoricalDecision
                              ? "approved an earlier commit"
                              : "approved these changes"
                            : timelineKind === "changes-requested"
                              ? isHistoricalDecision
                                ? "requested changes on an earlier commit"
                                : "requested changes"
                              : trimmedContent || "requested a review"}
                        </>
                      ) : null}
                    </span>
                    <span
                      className="ml-auto w-20 shrink-0 text-right text-xs text-muted-foreground/70"
                      title={formatExactTimestamp(item.createdAt)}
                    >
                      {relativeTime(item.createdAt)}
                    </span>
                  </div>
                  {activityContent ? (
                    <ProjectRichContent
                      className="mt-1 text-sm text-foreground/90"
                      content={activityContent}
                      tags={item.tags}
                    />
                  ) : null}
                  {item.anchor ? (
                    <button
                      aria-label={`Open ${item.anchor.path} ${item.anchor.side} line ${item.anchor.line} in Files changed`}
                      className="mt-1 inline-flex min-w-0 items-center gap-1 rounded-md bg-muted/65 px-1.5 py-0.5 font-mono text-2xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                      onClick={() => {
                        if (item.anchor) onOpenInlineComment?.(item.anchor);
                      }}
                      type="button"
                    >
                      <FileCode2 className="h-3 w-3 shrink-0" />
                      <span className="truncate">{item.anchor.path}</span>
                      <span className="shrink-0">
                        {item.anchor.side === "new" ? "+" : "-"}
                        {item.anchor.line}
                      </span>
                      {item.inlineCommentStatus === "outdated" ? (
                        <span className="shrink-0 text-destructive">
                          Outdated
                        </span>
                      ) : null}
                    </button>
                  ) : null}
                </div>
              </div>
            );
          })}
          <div className="flex min-h-12 items-start justify-start px-3 py-2.5">
            <PullRequestReviewCard
              onOpenTerminal={onOpenTerminal}
              project={project}
              pullRequest={pullRequest}
            />
          </div>
        </div>
        <h4 className="flex items-center gap-1.5 text-sm font-semibold text-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Add Your Comment
        </h4>
        <div data-testid="project-pull-request-comment-composer">
          <ForumComposer
            className="border border-border/60 bg-background/45"
            disabled={commentMutation.isPending}
            isSending={commentMutation.isPending}
            members={members}
            onSecondarySubmit={
              canRequestChanges ? handleChangeRequestSubmit : undefined
            }
            onSubmit={handleCommentSubmit}
            placeholder="Add a comment…"
            profiles={profiles}
            secondarySubmitLabel="Request changes"
          />
        </div>
      </section>
    </div>
  );
}

export function PullRequestsPanel({
  error,
  isLoading,
  mode = "conversation",
  onOpenInlineComment,
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
  onOpenInlineComment?: (anchor: ProjectPullRequestCommentAnchor) => void;
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
        onOpenInlineComment={onOpenInlineComment}
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
