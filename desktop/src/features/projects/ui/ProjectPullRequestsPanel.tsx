import {
  Check,
  GitMerge,
  GitPullRequest,
  GitPullRequestDraft,
  MessageSquare,
  UserPlus,
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
import {
  useApproveProjectPullRequestMutation,
  useRequestProjectPullRequestReviewMutation,
  useUpdateProjectPullRequestStatusMutation,
} from "@/features/projects/pullRequestReviews";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Markdown } from "@/shared/ui/markdown";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { MergePullRequestButton } from "./MergePullRequestButton";

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
          <span className="truncate font-medium text-foreground/80">
            {authorLabel}
          </span>
          <span>created {relativeCreatedAt(pullRequest.createdAt)}</span>
          <span className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs">
            Member
          </span>
          <span>·</span>
          <span>{pullRequest.status}</span>
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
            <span className="flex items-center gap-1 text-xs text-muted-foreground">
              <MessageSquare className="h-3.5 w-3.5" />
              {pullRequest.comments.length}
            </span>
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

/** Candidate reviewers: project owner, contributors, and PR recipients —
 * minus the PR author and anyone already requested. */
function reviewerCandidates(project: Project, pullRequest: ProjectPullRequest) {
  const requested = new Set(pullRequest.reviewers);
  const author = normalizePubkey(pullRequest.author);
  return [
    ...new Set(
      [project.owner, ...project.contributors, ...pullRequest.recipients].map(
        normalizePubkey,
      ),
    ),
  ].filter((pubkey) => pubkey !== author && !requested.has(pubkey));
}

function PullRequestReviewersRow({
  canRequest,
  profiles,
  project,
  pullRequest,
}: {
  canRequest: boolean;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const requestReviewMutation =
    useRequestProjectPullRequestReviewMutation(project);
  const candidates = reviewerCandidates(project, pullRequest);
  const approvedBy = new Set(
    pullRequest.approvals.map((approval) => normalizePubkey(approval.author)),
  );

  const handleRequest = React.useCallback(
    async (pubkey: string) => {
      try {
        await requestReviewMutation.mutateAsync({
          pullRequest,
          reviewers: [pubkey],
          reviewerLabel: labelForPubkey(pubkey, profiles),
        });
        toast.success("Review requested.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to request review.",
        );
      }
    },
    [profiles, pullRequest, requestReviewMutation],
  );

  if (pullRequest.reviewers.length === 0 && !canRequest) {
    return null;
  }

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5 px-1 text-xs text-muted-foreground">
      <span className="font-medium">Reviewers</span>
      {pullRequest.reviewers.map((pubkey) => {
        const profile = profileForPubkey(pubkey, profiles);
        const label = labelForPubkey(pubkey, profiles);
        const hasApproved = approvedBy.has(normalizePubkey(pubkey));
        return (
          <Tooltip key={pubkey}>
            <TooltipTrigger asChild>
              <span className="relative inline-flex">
                <UserAvatar
                  accent={profile?.isAgent === true}
                  avatarUrl={profile?.avatarUrl ?? null}
                  displayName={label}
                  size="xs"
                />
                {hasApproved ? (
                  <span className="-right-1 -bottom-1 absolute flex h-3.5 w-3.5 items-center justify-center rounded-full bg-green-600 text-white ring-2 ring-background">
                    <Check className="h-2.5 w-2.5" />
                  </span>
                ) : null}
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {label}
              {hasApproved ? " — approved" : " — review requested"}
            </TooltipContent>
          </Tooltip>
        );
      })}
      {canRequest && candidates.length > 0 ? (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              className="h-6 gap-1 px-2 text-2xs text-muted-foreground hover:text-foreground"
              disabled={requestReviewMutation.isPending}
              size="xs"
              type="button"
              variant="outline"
            >
              <UserPlus className="h-3 w-3" />
              Request
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="min-w-52">
            <DropdownMenuLabel>Request a review</DropdownMenuLabel>
            {candidates.map((pubkey) => {
              const profile = profileForPubkey(pubkey, profiles);
              const label = labelForPubkey(pubkey, profiles);
              return (
                <DropdownMenuItem
                  key={pubkey}
                  onSelect={() => {
                    void handleRequest(pubkey);
                  }}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <UserAvatar
                      accent={profile?.isAgent === true}
                      avatarUrl={profile?.avatarUrl ?? null}
                      displayName={label}
                      size="xs"
                    />
                    <span className="truncate">{label}</span>
                  </span>
                </DropdownMenuItem>
              );
            })}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
    </div>
  );
}

/** GitHub-style review box rendered in the conversation flow, above the
 * comment composer: reviewers on top, review state + actions below. */
function PullRequestReviewCard({
  profiles,
  project,
  pullRequest,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const identityQuery = useIdentityQuery();
  const statusMutation = useUpdateProjectPullRequestStatusMutation(project);
  const approveMutation = useApproveProjectPullRequestMutation(project);

  const viewerPubkey = identityQuery.data?.pubkey ?? null;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(pullRequest.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const canChangeStatus = Boolean(viewer) && (isAuthor || isOwner);
  const canRequestReview = canChangeStatus;
  const hasApproved = Boolean(
    viewer &&
      pullRequest.approvals.some(
        (approval) => normalizePubkey(approval.author) === viewer,
      ),
  );
  const canApprove =
    Boolean(viewer) &&
    !isAuthor &&
    !hasApproved &&
    (pullRequest.status === "Open" || pullRequest.status === "Draft");
  const canMerge =
    isOwner &&
    pullRequest.status === "Open" &&
    Boolean(pullRequest.branchName && pullRequest.commit);

  const handleStatusChange = React.useCallback(
    async (status: "open" | "draft") => {
      try {
        await statusMutation.mutateAsync({ pullRequest, status });
        toast.success(
          status === "draft"
            ? "Converted to draft."
            : "Marked as ready for review.",
        );
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to update status.",
        );
      }
    },
    [pullRequest, statusMutation],
  );

  const handleApprove = React.useCallback(async () => {
    try {
      await approveMutation.mutateAsync({ pullRequest });
      toast.success("Pull request approved.");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Failed to approve.",
      );
    }
  }, [approveMutation, pullRequest]);

  const approvalCount = pullRequest.approvals.length;
  const isDraft = pullRequest.status === "Draft";
  const reviewState = isDraft
    ? "This pull request is still a work in progress."
    : approvalCount > 0
      ? `Approved by ${pluralize(approvalCount, "reviewer")}.`
      : pullRequest.reviewers.length > 0
        ? "Review requested — no approvals yet."
        : "No reviews yet.";
  const reviewStateDetail = isDraft
    ? "Draft pull requests cannot be merged."
    : approvalCount === 0
      ? "Approvals from reviewers will show up here."
      : null;

  return (
    <div className="space-y-2.5 pt-3">
      <PullRequestReviewersRow
        canRequest={canRequestReview}
        profiles={profiles}
        project={project}
        pullRequest={pullRequest}
      />
      <div
        className={`min-w-0 space-y-2.5 rounded-xl px-3 py-2.5 ${
          isDraft
            ? "bg-muted/40"
            : approvalCount > 0
              ? "bg-green-600/10 dark:bg-green-500/10"
              : "bg-muted/40"
        }`}
      >
        <div className="flex min-w-0 items-start gap-2">
          {isDraft ? (
            <GitPullRequestDraft className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          ) : approvalCount > 0 ? (
            <span className="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-green-600 text-white">
              <Check className="h-3 w-3" />
            </span>
          ) : (
            <GitPullRequest className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
          )}
          <div className="min-w-0 flex-1">
            <p
              className={`text-sm font-medium ${
                approvalCount > 0
                  ? "text-green-700 dark:text-green-400"
                  : "text-foreground"
              }`}
            >
              {reviewState}
            </p>
            {reviewStateDetail ? (
              <p className="text-xs text-muted-foreground">
                {reviewStateDetail}
              </p>
            ) : null}
          </div>
        </div>
        {hasApproved ||
        canApprove ||
        canMerge ||
        (canChangeStatus && isDraft) ? (
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            {hasApproved ? (
              <span className="inline-flex items-center gap-1.5 rounded-md border border-green-600/40 px-2.5 py-1 text-xs font-medium text-green-600 dark:text-green-500">
                <Check className="h-3.5 w-3.5" />
                Approved
              </span>
            ) : null}
            {canApprove ? (
              <Button
                className="h-8 gap-1.5 bg-green-600 px-3.5 text-white shadow-sm hover:bg-green-700"
                disabled={approveMutation.isPending}
                onClick={() => {
                  void handleApprove();
                }}
                size="xs"
                type="button"
              >
                <Check className="h-3.5 w-3.5" />
                Approve
              </Button>
            ) : null}
            {canMerge ? (
              <MergePullRequestButton
                project={project}
                pullRequest={pullRequest}
              />
            ) : null}
            {canChangeStatus && isDraft ? (
              <Button
                className="h-7 gap-1.5 px-3"
                disabled={statusMutation.isPending}
                onClick={() => {
                  void handleStatusChange("open");
                }}
                size="xs"
                type="button"
                variant="secondary"
              >
                <GitPullRequest className="h-3.5 w-3.5" />
                Ready for review
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
      {canChangeStatus && pullRequest.status === "Open" ? (
        <p className="px-1 text-xs text-muted-foreground">
          Still in progress?{" "}
          <button
            className="font-medium underline-offset-2 hover:text-foreground hover:underline disabled:opacity-50"
            disabled={statusMutation.isPending}
            onClick={() => {
              void handleStatusChange("draft");
            }}
            type="button"
          >
            Convert to draft
          </button>
        </p>
      ) : null}
    </div>
  );
}

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

/** Right-hand meta column for the PR detail view: status, author, branches,
 * and dates. Review actions live inline in the conversation column. */
export function PullRequestMetaRail({
  profiles,
  project,
  pullRequest,
}: {
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
}) {
  const authorProfile = profileForPubkey(pullRequest.author, profiles);
  const authorLabel = labelForPubkey(pullRequest.author, profiles);
  const targetBranch = project.defaultBranch || "default branch";
  const sourceBranch = pullRequest.branchName || "unknown branch";
  const commitCount = Math.max(1, pullRequest.updateCount + 1);

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
  profiles,
  project,
  pullRequest,
}: {
  mode: PullRequestPanelMode;
  onOpenCommit?: (commitHash: string) => void;
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
          <h4 className="text-sm font-semibold text-foreground">Commits</h4>
          <article className="space-y-1">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <AuthorIdentity
                profiles={profiles}
                pubkey={pullRequest.author}
                role={compactDate(pullRequest.createdAt)}
              />
              {pullRequest.commit ? (
                <CommitHashChip
                  hash={pullRequest.commit}
                  onOpenCommit={onOpenCommit}
                />
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
      </div>
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
        <h4 className="flex items-center gap-1.5 text-sm font-semibold text-foreground">
          <MessageSquare className="h-3.5 w-3.5" />
          Add Your Comment
        </h4>
        {pullRequest.comments.length > 0 ? (
          <div className="space-y-3">
            {pullRequest.comments.map((item) => {
              // Approvals and review requests render as compact timeline
              // rows (GitHub-style) rather than full comment cards.
              if (item.isApproval || item.isReviewRequest) {
                return (
                  <div
                    className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground"
                    key={item.id}
                  >
                    {item.isApproval ? (
                      <Check className="h-3.5 w-3.5 shrink-0 text-green-600 dark:text-green-500" />
                    ) : (
                      <UserPlus className="h-3.5 w-3.5 shrink-0" />
                    )}
                    <span className="font-medium text-foreground">
                      {labelForPubkey(item.author, profiles)}
                    </span>
                    <span className="min-w-0 truncate">
                      {item.isApproval
                        ? "approved these changes"
                        : item.content.trim() || "requested a review"}
                    </span>
                    <span>· {compactDate(item.createdAt)}</span>
                  </div>
                );
              }
              return (
                <article key={item.id}>
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
        ) : (
          <p className="text-sm text-muted-foreground">No comments yet.</p>
        )}
        <PullRequestReviewCard
          profiles={profiles}
          project={project}
          pullRequest={pullRequest}
        />
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
