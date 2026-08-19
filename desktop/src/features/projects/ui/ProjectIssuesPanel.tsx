import { CircleCheck, CircleDot, CircleX, MessageSquare } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsManagedAgent } from "@/features/agent-memory/hooks";
import { ForumComposer } from "@/features/forum/ui/ForumComposer";
import {
  type ProjectIssue,
  type Repository as Project,
  useCreateProjectIssueCommentMutation,
  useProjectIssuesQuery,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { entityDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { IssueAssigneeFacepile, IssueAssigneesRow } from "./IssueAssigneesRow";
import {
  ProjectFeedRow,
  ProjectFeedRowCluster,
  ProjectFeedRowMonoCell,
} from "./ProjectFeedRow";
import { DiscussedInChannels } from "./DiscussionChannels";
import { ProjectIssueCommentTimeline } from "./ProjectIssueCommentTimeline";
import { ProjectOriginReference } from "./ProjectOriginReference";
import { OverviewRailSection } from "./ProjectOverviewPanel";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";
import { ProjectRichContent } from "./ProjectRichContent";
import { ShareLinkButton } from "./ShareLinkButton";

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

function IssueRow({
  issue,
  onOpen,
  profiles,
}: {
  issue: ProjectIssue;
  onOpen: () => void;
  profiles?: UserProfileLookup;
}) {
  const authorProfile = profiles?.[normalizePubkey(issue.author)];
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const status = issueStatusVisual(issue.status);

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
            pubkey={issue.author}
            showLabel={false}
          />
          <span className="truncate text-foreground/80">
            <span className="font-medium">{authorLabel}</span> created this
            issue
          </span>
          <span>·</span>
          <span>{issue.status}</span>
          {issue.labels.map((label) => (
            <span
              className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs"
              key={label}
            >
              {label}
            </span>
          ))}
        </>
      }
      eventId={issue.id}
      onOpen={onOpen}
      statusIcon={
        <status.icon className={`h-3.5 w-3.5 shrink-0 ${status.className}`} />
      }
      testId="project-issue-row"
      title={issue.title}
      trailing={
        <>
          <IssueAssigneeFacepile
            assignees={issue.assignees}
            profiles={profiles}
          />
          {issue.comments.length > 0 ? (
            <button
              aria-label={`View ${issue.comments.length} comments`}
              className="flex items-center gap-1 rounded-md text-xs text-muted-foreground hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onOpen}
              type="button"
            >
              <MessageSquare className="h-3.5 w-3.5" />
              {issue.comments.length}
            </button>
          ) : null}
          <ProjectFeedRowCluster>
            <ProjectFeedRowMonoCell
              label={`#${issue.id.slice(0, 8)}`}
              onClick={onOpen}
              title="View issue"
            />
          </ProjectFeedRowCluster>
          <span
            className="hidden w-20 shrink-0 text-right text-xs text-muted-foreground sm:block"
            data-testid="project-issue-row-date"
            title={new Date(issue.createdAt * 1_000).toLocaleString()}
          >
            {relativeTime(issue.createdAt)}
          </span>
        </>
      }
    />
  );
}

/** Full issue conversation and comment composer. */
export function ProjectIssueDetail({
  issue,
  profiles,
  project,
  stackMetaRail = false,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
  stackMetaRail?: boolean;
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
    <div
      className={cn(
        "grid",
        !stackMetaRail && "xl:grid-cols-[minmax(0,1fr)_18rem]",
      )}
    >
      <div className="min-w-0">
        <header className="space-y-3 p-4">
          <div className="min-w-0">
            <p className="flex flex-wrap items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <CircleDot className="h-3.5 w-3.5" />
              Issue from {authorLabel}
              <ProjectOriginReference
                agentName={issue.originAgentName}
                channelId={issue.channelId}
              />
            </p>
            <h3 className="mt-1 line-clamp-2 text-base font-semibold text-foreground">
              {issue.title}{" "}
              <span className="font-normal text-muted-foreground">
                #{issue.id.slice(0, 8)}
              </span>
              <ShareLinkButton
                className="ml-1 inline-flex h-6 w-6 align-text-bottom"
                label="Copy issue link"
                link={issueShareLink(issue)}
                testId="project-issue-copy-link"
              />
            </h3>
          </div>
          {issue.content ? (
            <ProjectRichContent content={issue.content} tags={issue.tags} />
          ) : null}
        </header>

        <section className="space-y-3 p-4">
          <DiscussedInChannels
            entityLabel="this issue"
            query={entityDiscussionQuery(issue.id)}
            testId="issue-discussed-in"
          />
          <ProjectIssueCommentTimeline
            comments={issue.comments}
            key={issue.id}
            profiles={profiles}
          />
          <div data-testid="project-issue-comment-composer">
            <ForumComposer
              className="border border-border/60 bg-background/45"
              disabled={commentMutation.isPending}
              isSending={commentMutation.isPending}
              members={members}
              onSubmit={handleCommentSubmit}
              placeholder="Add a comment…"
              profiles={profiles}
            />
          </div>
        </section>
      </div>

      <IssueMetaRail
        issue={issue}
        profiles={profiles}
        project={project}
        stacked={stackMetaRail}
      />
    </div>
  );
}

/** Right-hand meta column for the issue detail view: status, assignees,
 * author, labels, and dates — keeps the conversation column focused. */
function IssueMetaRail({
  issue,
  profiles,
  project,
  stacked = false,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
  stacked?: boolean;
}) {
  const identityQuery = useIdentityQuery();
  const authorProfile = profiles?.[normalizePubkey(issue.author)];
  const authorLabel = resolveUserLabel({ profiles, pubkey: issue.author });
  const status = issueStatusVisual(issue.status);
  const viewerPubkey = identityQuery.data?.pubkey;
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const isAuthor = viewer === normalizePubkey(issue.author);
  const isOwner = viewer === normalizePubkey(project.owner);
  const isManagedAgentOwner = useIsManagedAgent(project.owner) === true;
  // Same trust rule as parsing (assigneesForIssue): the issue author or
  // repo owner (directly or via a managed agent) can assign anyone;
  // everyone else who is signed in may still self-assign.
  const canAssignOthers =
    Boolean(viewer) && (isAuthor || isOwner || isManagedAgentOwner);

  return (
    <aside
      className={cn(
        "space-y-6 border-border/60 p-4",
        stacked ? "border-t" : "border-t xl:border-l xl:border-t-0",
      )}
    >
      <OverviewRailSection title="Status">
        <span
          className={`inline-flex items-center gap-1.5 rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium ${status.className}`}
        >
          <status.icon className="h-3.5 w-3.5" />
          {issue.status}
        </span>
      </OverviewRailSection>
      {issue.assignees.length > 0 || viewer ? (
        <OverviewRailSection title="Assignees">
          <IssueAssigneesRow
            canAssignOthers={canAssignOthers}
            issue={issue}
            profiles={profiles}
            project={project}
            signAsManagedOwner={isManagedAgentOwner && !isOwner}
            viewerPubkey={viewer}
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
          pubkey={issue.author}
        />
      </OverviewRailSection>
      {issue.labels.length > 0 ? (
        <OverviewRailSection title="Labels">
          <div className="flex flex-wrap gap-1.5">
            {issue.labels.map((label) => (
              <span
                className="rounded-full border border-border/60 px-1.5 py-0.5 text-2xs text-muted-foreground"
                key={label}
              >
                {label}
              </span>
            ))}
          </div>
        </OverviewRailSection>
      ) : null}
      <OverviewRailSection title="Activity">
        <dl className="space-y-1.5 text-xs text-muted-foreground">
          <div className="flex items-center justify-between gap-3">
            <dt>Created</dt>
            <dd className="font-medium text-foreground">
              {relativeTime(issue.createdAt)}
            </dd>
          </div>
          <div className="flex items-center justify-between gap-3">
            <dt>Updated</dt>
            <dd className="font-medium text-foreground">
              {relativeTime(issue.updatedAt)}
            </dd>
          </div>
        </dl>
      </OverviewRailSection>
    </aside>
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
      <ProjectIssueDetail
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
