import {
  ChevronDown,
  ChevronRight,
  CircleCheck,
  CircleDot,
  MessageSquare,
} from "lucide-react";
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
import {
  canSubmitProjectIssueVerdict,
  MAX_REJECTION_REASON_LENGTH,
  canWriteMyBuzzWorkflowStatus,
  type MyBuzzWorkflowStatusState,
  useSubmitProjectIssueVerdictMutation,
  useUpdateProjectIssueStatusMutation,
} from "@/features/projects/issueStatus";
import { entityDiscussionQuery } from "@/features/projects/lib/discussionChannels";
import { issueShareLink } from "@/features/projects/lib/projectShareLinks";
import { relativeTime } from "@/features/projects/lib/projectsViewHelpers";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ChannelMember } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
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
  return "text-green-500";
}

export function reviewSectionState(
  issue: Pick<ProjectIssue, "currentReview">,
  viewer: string | null,
  submissionState: "idle" | "sent" | "failed" = "idle",
) {
  if (!issue.currentReview) return null;
  const verdict = issue.currentReview.verdict;
  const confirmation = verdict?.confirmation;
  const message = confirmation
    ? verdict.kind === "accepted"
      ? "Abnahme übernommen"
      : "Zur Nacharbeit zurückgegeben"
    : verdict || submissionState === "sent"
      ? "Urteil gesendet – Workflow-Bestätigung ausstehend"
      : submissionState === "failed"
        ? "Urteil konnte nicht gesendet werden. Bitte erneut versuchen."
        : null;
  return {
    ...issue.currentReview,
    canSubmit:
      canSubmitProjectIssueVerdict(issue, viewer) &&
      verdict === null &&
      submissionState !== "sent",
    message,
  };
}

const ISSUE_STATUS_SECTIONS = [
  { status: "Triage", label: "Triage", terminal: false },
  { status: "Backlog", label: "Backlog", terminal: false },
  { status: "In Development", label: "In Development", terminal: false },
  { status: "Implemented", label: "Implemented", terminal: false },
  { status: "Code-QS", label: "Code-QS", terminal: false },
  { status: "To Be Published", label: "To Be Published", terminal: false },
  { status: "Ready for Test", label: "Ready for Test", terminal: false },
  { status: "Done", label: "Done", terminal: true },
] as const;

const ISSUE_ROWS_PER_GROUP_STORAGE_KEY = "buzz.projects.issueRows";

function readIssueRowsPerGroup(): number {
  const stored = Number(
    globalThis.localStorage?.getItem(ISSUE_ROWS_PER_GROUP_STORAGE_KEY),
  );
  return stored === 10 || stored === 20 ? stored : 5;
}

function issueStatusVisual(status: ProjectIssue["status"]) {
  if (status === "Done") {
    return { className: "text-purple-400", icon: CircleCheck };
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
              label={`ISS-${issue.id.slice(0, 8).toUpperCase()}`}
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

function IssueActivity({ issue }: { issue: ProjectIssue }) {
  return (
    <section className="space-y-3 p-4" data-testid="project-issue-activity">
      <p className="text-sm text-muted-foreground">
        Workflow history is read-only and does not grant lifecycle authority.
      </p>
      <ol className="space-y-2 border-l border-border/60 pl-3 text-sm">
        {issue.activity.map((entry) => (
          <li key={entry.id}>
            <p className="text-foreground">{entry.text}</p>
            <p className="text-xs text-muted-foreground">
              {relativeTime(entry.createdAt)}
            </p>
          </li>
        ))}
      </ol>
    </section>
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
                ISS-{issue.id.slice(0, 8).toUpperCase()}
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

        <Tabs className="px-4 pb-4" defaultValue="discussion">
          <TabsList>
            <TabsTrigger value="discussion">Discussion</TabsTrigger>
            <TabsTrigger value="activity">Activity</TabsTrigger>
          </TabsList>
          <TabsContent value="discussion">
            <section className="space-y-3 py-4">
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
          </TabsContent>
          <TabsContent value="activity">
            <IssueActivity issue={issue} />
          </TabsContent>
        </Tabs>
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

function IssueStatusPicker({
  issue,
  project,
}: {
  issue: ProjectIssue;
  project: Project;
}) {
  const { isPending, mutateAsync: updateIssueStatus } =
    useUpdateProjectIssueStatusMutation(project);
  const [state, setState] = React.useState<MyBuzzWorkflowStatusState>("triage");
  const [reason, setReason] = React.useState("");
  const requiresReason = ["triage", "backlog", "ready-for-test"].includes(
    state,
  );

  const handleSelect = React.useCallback(async () => {
    try {
      await updateIssueStatus({ issue, reason, state });
      toast.success("Workflow status updated.");
      setReason("");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to update workflow status.",
      );
    }
  }, [issue, reason, state, updateIssueStatus]);

  return (
    <div className="space-y-2">
      <select
        aria-label="Change MyBuzz workflow status"
        className="h-8 w-full rounded-md border border-border/60 bg-transparent px-2 text-xs text-foreground"
        disabled={isPending}
        onChange={(event) =>
          setState(event.target.value as MyBuzzWorkflowStatusState)
        }
        value={state}
      >
        <option value="triage">Triage</option>
        <option value="backlog">Backlog</option>
        <option value="in-development">In Development</option>
        <option value="implemented">Implemented</option>
        <option value="code-qs">Code-QS</option>
        <option value="to-be-published">To Be Published</option>
        <option value="ready-for-test">Ready for Test</option>
      </select>
      <textarea
        aria-label="Workflow status reason"
        className="min-h-16 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
        disabled={isPending}
        onChange={(event) => setReason(event.target.value)}
        placeholder={requiresReason ? "Reason required" : "Reason (optional)"}
        value={reason}
      />
      <button
        className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
        disabled={isPending || (requiresReason && !reason.trim())}
        onClick={() => void handleSelect()}
        type="button"
      >
        Set workflow status
      </button>
    </div>
  );
}

function IssueReviewSection({
  issue,
  project,
  viewer,
}: {
  issue: ProjectIssue;
  project: Project;
  viewer: string | null;
}) {
  const [reason, setReason] = React.useState("");
  const [submissionState, setSubmissionState] = React.useState<
    "idle" | "sent" | "failed"
  >("idle");
  const { isPending, mutateAsync: submitVerdict } =
    useSubmitProjectIssueVerdictMutation(project);
  const review = reviewSectionState(issue, viewer, submissionState);

  if (!review) return null;

  const submit = async (verdict: "accepted" | "rejected") => {
    if (!review.canSubmit) return;
    if (
      verdict === "accepted" &&
      !globalThis.confirm(
        `Done for review ${review.id}?\n\nImmutable target: ${review.target}`,
      )
    ) {
      return;
    }
    try {
      await submitVerdict({
        issue,
        ...(verdict === "rejected" ? { reason } : {}),
        verdict,
      });
      setSubmissionState("sent");
      toast.success("Urteil gesendet – Workflow-Bestätigung ausstehend");
    } catch (error) {
      setSubmissionState("failed");
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to send human verdict.",
      );
    }
  };

  return (
    <OverviewRailSection title="Review">
      <dl className="space-y-2 text-xs">
        <div>
          <dt className="text-muted-foreground">Review-ID</dt>
          <dd className="break-all font-medium text-foreground">{review.id}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Target</dt>
          <dd className="break-words text-foreground">{review.target}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Evidence</dt>
          <dd className="break-words text-foreground">{review.evidence}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Test</dt>
          <dd className="break-words text-foreground">{review.test}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">Known limitations</dt>
          <dd className="break-words text-foreground">{review.limitations}</dd>
        </div>
      </dl>
      {review.message ? (
        <p className="mt-3 text-xs font-medium text-foreground" role="status">
          {review.message}
        </p>
      ) : null}
      {review.canSubmit ? (
        <div className="mt-3 space-y-2">
          <button
            className="rounded-md bg-primary px-2.5 py-1 text-xs font-medium text-primary-foreground disabled:opacity-60"
            disabled={isPending}
            onClick={() => void submit("accepted")}
            type="button"
          >
            Done
          </button>
          <label
            className="block text-xs text-muted-foreground"
            htmlFor="review-rejection-reason"
          >
            Ablehnen mit Grund
          </label>
          <textarea
            className="min-h-20 w-full rounded-md border border-border/60 bg-background p-2 text-xs text-foreground"
            disabled={isPending}
            id="review-rejection-reason"
            maxLength={MAX_REJECTION_REASON_LENGTH}
            onChange={(event) => setReason(event.target.value)}
            value={reason}
          />
          <button
            className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
            disabled={isPending || !reason.trim()}
            onClick={() => void submit("rejected")}
            type="button"
          >
            Ablehnen
          </button>
        </div>
      ) : !review.verdict && submissionState !== "sent" ? (
        <p className="mt-3 text-xs text-muted-foreground">
          Review data is read-only for this identity.
        </p>
      ) : null}
    </OverviewRailSection>
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
  const canChangeStatus = canWriteMyBuzzWorkflowStatus(viewer);

  return (
    <aside
      className={cn(
        "space-y-6 border-border/60 p-4",
        stacked ? "border-t" : "border-t xl:border-l xl:border-t-0",
      )}
    >
      <OverviewRailSection title="Status">
        {canChangeStatus ? (
          <IssueStatusPicker issue={issue} project={project} />
        ) : (
          <span
            className={`inline-flex items-center gap-1.5 rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium ${status.className}`}
            data-testid="project-issue-status"
          >
            <status.icon className="h-3.5 w-3.5" />
            {issue.status}
          </span>
        )}
      </OverviewRailSection>
      <IssueReviewSection
        issue={issue}
        key={`${issue.id}:${issue.currentReview?.id ?? "none"}`}
        project={project}
        viewer={viewer}
      />
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
      <OverviewRailSection title="Metadata">
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
  const [rowsPerGroup, setRowsPerGroup] = React.useState(readIssueRowsPerGroup);
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

  const changeRowsPerGroup = (value: number) => {
    setRowsPerGroup(value);
    globalThis.localStorage?.setItem(
      ISSUE_ROWS_PER_GROUP_STORAGE_KEY,
      String(value),
    );
  };

  return (
    <div className="space-y-4 p-2">
      <div className="flex items-center justify-end gap-2 px-2 text-xs text-muted-foreground">
        <label htmlFor="channel-issue-rows">Rows per status</label>
        <select
          className="h-8 rounded-md bg-transparent px-2 text-xs text-foreground outline-hidden hover:bg-muted/50 focus:ring-1 focus:ring-ring"
          id="channel-issue-rows"
          onChange={(event) => changeRowsPerGroup(Number(event.target.value))}
          value={rowsPerGroup}
        >
          <option value={5}>5</option>
          <option value={10}>10</option>
          <option value={20}>20</option>
        </select>
      </div>
      {ISSUE_STATUS_SECTIONS.map(({ label, status, terminal }) => {
        const sectionIssues = issues.filter((issue) => issue.status === status);
        if (sectionIssues.length === 0) return null;
        return (
          <IssueStatusSection
            issues={sectionIssues}
            key={status}
            label={label}
            onOpen={onSelectedIssueIdChange}
            profiles={profiles}
            rowsPerGroup={rowsPerGroup}
            terminal={terminal}
          />
        );
      })}
    </div>
  );
}

function IssueStatusSection({
  issues,
  label,
  onOpen,
  profiles,
  rowsPerGroup,
  terminal,
}: {
  issues: ProjectIssue[];
  label: string;
  onOpen: (id: string | null) => void;
  profiles?: UserProfileLookup;
  rowsPerGroup: number;
  terminal: boolean;
}) {
  const [expanded, setExpanded] = React.useState(!terminal);
  const [showAll, setShowAll] = React.useState(false);
  const visibleIssues = expanded
    ? showAll
      ? issues
      : issues.slice(0, rowsPerGroup)
    : [];
  const remaining = issues.length - visibleIssues.length;

  return (
    <section>
      <div className="flex items-center justify-between px-2 py-1.5">
        {terminal ? (
          <button
            aria-expanded={expanded}
            className="flex items-center gap-1 text-xs font-semibold text-muted-foreground hover:text-foreground"
            onClick={() => setExpanded((value) => !value)}
            type="button"
          >
            {expanded ? (
              <ChevronDown className="h-3.5 w-3.5" />
            ) : (
              <ChevronRight className="h-3.5 w-3.5" />
            )}
            {label} · {issues.length}
          </button>
        ) : (
          <h3 className="text-xs font-semibold text-muted-foreground">
            {label} · {issues.length}
          </h3>
        )}
        {expanded && remaining > 0 ? (
          <button
            aria-expanded={showAll}
            className="text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setShowAll(true)}
            type="button"
          >
            Show {remaining} more
          </button>
        ) : expanded && showAll ? (
          <button
            aria-expanded={showAll}
            className="text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setShowAll(false)}
            type="button"
          >
            Show less
          </button>
        ) : null}
      </div>
      {expanded ? (
        <div className="divide-y divide-border/50 overflow-hidden rounded-lg border border-border/50">
          {visibleIssues.map((issue) => (
            <IssueRow
              issue={issue}
              key={issue.id}
              onOpen={() => onOpen(issue.id)}
              profiles={profiles}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}
