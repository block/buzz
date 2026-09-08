import {
  Check,
  ChevronDown,
  ChevronUp,
  FileCode2,
  GitBranch,
  History,
  MessageSquare,
  TriangleAlert,
  UserPlus,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { DiscussedInChannels } from "./DiscussionChannels";
import { ProjectOriginReference } from "./ProjectOriginReference";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type ProjectPullRequest,
  type Repository as Project,
  type ProjectPullRequestCommentAnchor,
  useCreateProjectPullRequestCommentMutation,
} from "@/features/projects/hooks";
import { projectPullRequestCommentTimelineKind } from "@/features/projects/projectPullRequests.mjs";
import { entityDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import { selectionItemFromReview } from "@/features/projects/lib/projectSelection";
import { pullRequestShareLink } from "@/features/projects/lib/projectShareLinks";
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
import { ProjectDetailSection } from "./ProjectDetailSection";
import type { OpenMergeRecoveryTerminal } from "./MergePullRequestButton";
import {
  ProfileAuthorName,
  ProfileIdentityButton,
} from "./ProjectProfileIdentity";
import { ProjectRichContent } from "./ProjectRichContent";
import { PullRequestMetaHeader } from "./PullRequestMetaRail";
import { PullRequestReviewCard } from "./PullRequestReviewCard";
import { ShareLinkButton } from "./ShareLinkButton";
import { PROJECT_DETAIL_READING_COLUMN_CLASS } from "./projectPanelStyles";
import {
  ProjectStatusProgressIcon,
  type ProjectStatusProgressState,
} from "./ProjectStatusProgressIcon";
import { ProjectWorkItemGroup } from "./ProjectWorkItemGroup";
import { ProjectWorkItemRow } from "./ProjectWorkItemRow";
import { PullRequestsPanelSurface } from "./PullRequestsPanelSurface";

export { PullRequestMetaHeader } from "./PullRequestMetaRail";

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

function pullRequestStatusClassName(status: ProjectPullRequest["status"]) {
  if (status === "Closed") return "text-destructive";
  if (status === "Draft") return "text-muted-foreground";
  if (status === "Merged") return "text-purple-400";
  return "text-green-500";
}

const PULL_REQUEST_STATUS_ORDER: readonly ProjectPullRequest["status"][] = [
  "Open",
  "Draft",
  "Merged",
  "Closed",
];

function pullRequestProgressState(
  status: ProjectPullRequest["status"],
): ProjectStatusProgressState {
  if (status === "Draft") return "queued";
  if (status === "Merged") return "completed";
  if (status === "Closed") return "canceled";
  return "review";
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
            authored
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
        <>
          {hash ? (
            <ProjectFeedRowCluster>
              <ProjectFeedRowMonoCell
                label={hash.slice(0, 7)}
                onClick={openCommit}
                title={`View commit ${hash.slice(0, 7)}`}
              />
              <CopyCommitHashButton hash={hash} />
            </ProjectFeedRowCluster>
          ) : null}
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
            data-testid="project-pull-request-commit-row-date"
            title={formatExactTimestamp(createdAt)}
          >
            {relativeTime(createdAt)}
          </span>
        </>
      }
    />
  );
}

function reviewSelectionItem(
  project: Project,
  pullRequest: ProjectPullRequest,
) {
  return selectionItemFromReview({
    author: pullRequest.author,
    channelId: pullRequest.channelId ?? project.channelId,
    id: pullRequest.id,
    shareLink: pullRequestShareLink(pullRequest),
    title: pullRequest.title,
  });
}

function PullRequestRow({
  onOpen,
  profiles,
  project,
  pullRequest,
  rangeItems,
}: {
  onOpen: () => void;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
  rangeItems: ReturnType<typeof reviewSelectionItem>[];
}) {
  const authorProfile = profileForPubkey(pullRequest.author, profiles);
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const statusClassName = pullRequestStatusClassName(pullRequest.status);

  return (
    <ProjectWorkItemRow
      eventId={pullRequest.id}
      identifier={`#${pullRequest.id.slice(0, 8)}`}
      identifierTitle="View review"
      metadata={
        pullRequest.branchName ? (
          <span className="inline-flex min-w-0 items-center gap-1">
            <GitBranch className="h-3 w-3 shrink-0" />
            <span className="truncate">{pullRequest.branchName}</span>
          </span>
        ) : undefined
      }
      onOpen={onOpen}
      selection={{
        item: reviewSelectionItem(project, pullRequest),
        rangeItems,
      }}
      statusIcon={
        <ProjectStatusProgressIcon
          aria-label={pullRequest.status}
          className={`h-3.5 w-3.5 shrink-0 ${statusClassName}`}
          state={pullRequestProgressState(pullRequest.status)}
        />
      }
      testId="project-pull-request-row"
      title={pullRequest.title}
      trailing={
        <>
          <span className="flex w-8 shrink-0 justify-end">
            <button
              aria-label={
                pullRequest.comments.length > 0
                  ? `View ${pullRequest.comments.length} comments`
                  : "View comments"
              }
              className={`flex items-center gap-1 rounded-md text-xs hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring ${
                pullRequest.comments.length > 0
                  ? "text-muted-foreground/60"
                  : "text-muted-foreground/45"
              }`}
              data-testid="project-pull-request-comments"
              onClick={onOpen}
              type="button"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {pullRequest.comments.length}
            </button>
          </span>
          <ProfileIdentityButton
            avatarClassName="shrink-0"
            avatarSize="xs"
            avatarUrl={authorProfile?.avatarUrl ?? null}
            isAgent={authorProfile?.isAgent === true}
            label={authorLabel}
            pubkey={pullRequest.author}
            showLabel={false}
          />
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground/55 sm:block"
            data-testid="project-pull-request-row-date"
            title={formatExactTimestamp(pullRequest.createdAt)}
          >
            {relativeTime(pullRequest.createdAt)}
          </span>
        </>
      }
    />
  );
}

/** GitHub-style PR title line, rendered as the top of the review detail
 * card. Status, branches, and dates sit in the meta header below. */
export function PullRequestDetailHeader({
  pullRequest,
}: {
  pullRequest: ProjectPullRequest;
}) {
  return (
    <header className="min-w-0 space-y-2 px-6 pb-3 pt-5">
      <h3 className="line-clamp-2 min-w-0 text-lg font-semibold leading-6 text-foreground">
        {pullRequest.title}{" "}
        <span className="font-normal text-muted-foreground">
          #{pullRequest.id.slice(0, 8)}
        </span>
        <ShareLinkButton
          className="ml-1 inline-flex h-7 w-7 align-text-bottom"
          label="Copy review link"
          link={pullRequestShareLink(pullRequest)}
          testId="project-pull-request-copy-link"
        />
      </h3>
      <p
        className="flex flex-wrap items-center gap-x-1 gap-y-1 text-xs text-muted-foreground"
        data-testid="project-pull-request-detail-metadata"
      >
        <span data-project-metadata-phrase>Review opened</span>
        <span
          className="shrink-0 whitespace-nowrap"
          data-project-metadata-phrase
          title={formatExactTimestamp(pullRequest.createdAt)}
        >
          {relativeTime(pullRequest.createdAt)}
        </span>
        <ProjectOriginReference
          agentName={pullRequest.originAgentName}
          channelId={pullRequest.channelId}
        />
      </p>
    </header>
  );
}

/** Full pull-request conversation, review actions, and comment composer. */
export function ProjectPullRequestDetail({
  diffStats,
  filesChanged,
  filesCount,
  forceOpenFiles = false,
  onOpenInlineComment,
  onOpenCommit,
  onOpenTerminal,
  profiles,
  project,
  pullRequest,
}: {
  diffStats?: { additions: number; deletions: number } | null;
  filesChanged?: React.ReactNode;
  filesCount?: number;
  forceOpenFiles?: boolean;
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
  const [filesOpen, setFilesOpen] = React.useState(forceOpenFiles);
  const [filesOpenForId, setFilesOpenForId] = React.useState(pullRequest.id);
  if (filesOpenForId !== pullRequest.id) {
    setFilesOpenForId(pullRequest.id);
    setFilesOpen(forceOpenFiles);
  }
  const members = React.useMemo(
    () => pullRequestMembers(project, pullRequest, profiles),
    [profiles, project, pullRequest],
  );
  React.useEffect(() => {
    if (forceOpenFiles) setFilesOpen(true);
  }, [forceOpenFiles]);
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

  const commitCount = Math.max(1, pullRequest.updates.length + 1);
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
    <div
      className={PROJECT_DETAIL_READING_COLUMN_CLASS}
      data-project-detail-panel
      data-testid="project-pull-request-detail"
    >
      <PullRequestDetailHeader pullRequest={pullRequest} />
      <PullRequestMetaHeader
        diffStats={diffStats}
        profiles={profiles}
        project={project}
        pullRequest={pullRequest}
      />
      {pullRequest.content || pullRequest.updates.length > 0 ? (
        <ProjectDetailSection defaultOpen title="Description">
          {pullRequest.content ? (
            <ProjectRichContent
              content={pullRequest.content}
              tags={pullRequest.tags}
            />
          ) : null}
          {pullRequest.updates.length > 0 ? (
            <div
              className={pullRequest.content ? "mt-4 space-y-4" : "space-y-4"}
            >
              <h4 className="text-sm font-semibold text-foreground">Updates</h4>
              {pullRequest.updates.map((update) => (
                <article className="space-y-1" key={update.id}>
                  <div className="flex min-w-0 items-center justify-between gap-3">
                    <AuthorIdentity
                      avatarSize="sm"
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
            </div>
          ) : null}
        </ProjectDetailSection>
      ) : null}
      {filesChanged ? (
        <ProjectDetailSection
          count={filesCount}
          defaultOpen={false}
          onOpenChange={setFilesOpen}
          open={filesOpen}
          testId="project-detail-section-files"
          title="Files changed"
        >
          <div className="-mx-6">{filesChanged}</div>
        </ProjectDetailSection>
      ) : null}
      <ProjectDetailSection
        count={commitCount}
        defaultOpen={false}
        title="Commits"
      >
        <div className="-mx-6">
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
              message={update.content.trim() || "Updated review branch"}
              onOpenCommit={onOpenCommit}
              profiles={profiles}
            />
          ))}
        </div>
      </ProjectDetailSection>
      <ProjectDetailSection defaultOpen={false} title="Checks">
        <p className="text-sm text-muted-foreground">
          No checks have been reported for this review yet.
        </p>
      </ProjectDetailSection>
      <ProjectDetailSection defaultOpen title="Activity">
        <div className="space-y-3">
          <DiscussedInChannels
            entityLabel="this review"
            originChannelId={pullRequest.channelId}
            originCreatedAt={pullRequest.createdAt}
            originPubkey={pullRequest.author}
            query={entityDiscussionQuery(pullRequest.id)}
            testId="pull-request-discussed-in"
          />
          <div className="group/timeline -mx-6 overflow-hidden">
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
        </div>
      </ProjectDetailSection>
      <div
        className="px-6 pb-6 pt-4"
        data-testid="project-pull-request-comment-composer"
      >
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
    </div>
  );
}

export function PullRequestsPanel({
  diffStats,
  error,
  filesChanged,
  filesCount,
  forceOpenFiles,
  isLoading,
  onOpenInlineComment,
  onOpenCommit,
  onOpenTerminal,
  onSelectedPullRequestIdChange,
  profiles,
  project,
  pullRequests,
  selectedPullRequest,
}: {
  diffStats?: { additions: number; deletions: number } | null;
  error: unknown;
  filesChanged?: React.ReactNode;
  filesCount?: number;
  forceOpenFiles?: boolean;
  isLoading: boolean;
  onOpenInlineComment?: (anchor: ProjectPullRequestCommentAnchor) => void;
  onOpenCommit?: (commitHash: string) => void;
  onOpenTerminal?: OpenMergeRecoveryTerminal;
  onSelectedPullRequestIdChange: (id: string | null) => void;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequests: ProjectPullRequest[];
  selectedPullRequest: ProjectPullRequest | null;
}) {
  const groups = PULL_REQUEST_STATUS_ORDER.map((status) => ({
    items: pullRequests.filter((pullRequest) => pullRequest.status === status),
    status,
  })).filter((group) => group.items.length > 0);
  const rangeItems = pullRequests.map((pullRequest) =>
    reviewSelectionItem(project, pullRequest),
  );

  return (
    <PullRequestsPanelSurface
      detail={
        selectedPullRequest ? (
          <ProjectPullRequestDetail
            diffStats={diffStats}
            filesChanged={filesChanged}
            filesCount={filesCount}
            forceOpenFiles={forceOpenFiles}
            onOpenInlineComment={onOpenInlineComment}
            onOpenCommit={onOpenCommit}
            onOpenTerminal={onOpenTerminal}
            profiles={profiles}
            project={project}
            pullRequest={selectedPullRequest}
          />
        ) : null
      }
      error={error}
      isLoading={isLoading}
      list={
        <div>
          {groups.map(({ items, status }) => {
            return (
              <ProjectWorkItemGroup
                count={items.length}
                icon={
                  <ProjectStatusProgressIcon
                    className={`h-4 w-4 ${pullRequestStatusClassName(status)}`}
                    state={pullRequestProgressState(status)}
                  />
                }
                items={items.map((pullRequest) =>
                  reviewSelectionItem(project, pullRequest),
                )}
                key={status}
                label={status}
              >
                {items.map((pullRequest) => (
                  <PullRequestRow
                    key={pullRequest.id}
                    onOpen={() => onSelectedPullRequestIdChange(pullRequest.id)}
                    profiles={profiles}
                    project={project}
                    pullRequest={pullRequest}
                    rangeItems={rangeItems}
                  />
                ))}
              </ProjectWorkItemGroup>
            );
          })}
        </div>
      }
      pullRequests={pullRequests}
      selectedPullRequest={selectedPullRequest}
    />
  );
}
