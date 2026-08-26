import { Check, Search, UserPlus, Users, X } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import type { Repository as Project } from "@/features/projects/hooks";
import {
  useAssignProjectIssueMutation,
  useUnassignProjectIssueMutation,
} from "@/features/projects/issueAssignments";
import type { ProjectIssue } from "@/features/projects/projectIssues.mjs";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { PROJECT_CONTEXT_ACTION_BUTTON_CLASS } from "./projectContextActionStyles";

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

function assigneeSearchLabel(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

/** Compact overlapping assignee avatars for issue list rows. */
export function IssueAssigneeFacepile({
  assignees,
  profiles,
}: {
  assignees: string[];
  profiles?: UserProfileLookup;
}) {
  if (assignees.length === 0) return null;
  return (
    <span
      className="flex shrink-0 -space-x-1"
      data-testid="project-issue-assignees"
    >
      {assignees.slice(0, 3).map((pubkey) => {
        const profile = profileForPubkey(pubkey, profiles);
        const label = labelForPubkey(pubkey, profiles);
        return (
          <span
            className="inline-flex rounded-full ring-1 ring-background"
            key={pubkey}
            title={`Assigned to ${label}`}
          >
            <UserAvatar
              accent={profile?.isAgent === true}
              avatarUrl={profile?.avatarUrl ?? null}
              displayName={label}
              size="xs"
            />
          </span>
        );
      })}
    </span>
  );
}

/** Assignee avatars and the assignment picker for an issue.
 *
 * The issue author, repo owner, or managed-agent owner
 * (`canAssignOthers`) get the full people/agent picker. Everyone else
 * who is signed in gets a self-assign button — readers trust an
 * assignment whose only assignee is its signer (see `projectIssues.mjs`).
 */
export function IssueAssigneesRow({
  canAssignOthers,
  contextActions = false,
  issue,
  profiles,
  project,
  signAsManagedOwner,
  showAssignees = true,
  showSelfAssignmentState = false,
  testIdPrefix = "project-issue",
  viewerPubkey,
}: {
  canAssignOthers: boolean;
  contextActions?: boolean;
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
  signAsManagedOwner: boolean;
  showAssignees?: boolean;
  showSelfAssignmentState?: boolean;
  testIdPrefix?: string;
  viewerPubkey: string | null;
}) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [assigneeQuery, setAssigneeQuery] = React.useState("");
  const assignmentOperationInFlightRef = React.useRef(false);
  const assignMutation = useAssignProjectIssueMutation(project);
  const unassignMutation = useUnassignProjectIssueMutation(project);
  const deferredAssigneeQuery = React.useDeferredValue(assigneeQuery.trim());
  const currentAssignees = React.useMemo(
    () => new Set(issue.assignees.map(normalizePubkey)),
    [issue.assignees],
  );
  const userSearchQuery = useUserSearchQuery(deferredAssigneeQuery, {
    allowEmpty: true,
    enabled: canAssignOthers && pickerOpen,
    limit: 50,
  });
  const isArchivedDiscovery = useIsArchivedPredicate();
  // Unlike PR reviewers, the issue author stays in the candidate list —
  // self-assignment is normal issue-tracker behavior.
  const candidates = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter((user) => {
        const pubkey = normalizePubkey(user.pubkey);
        return !currentAssignees.has(pubkey) && !isArchivedDiscovery(pubkey);
      }),
    [currentAssignees, isArchivedDiscovery, userSearchQuery.data],
  );
  const viewer = viewerPubkey ? normalizePubkey(viewerPubkey) : null;
  const operationSigner = viewer ?? project.owner;

  const handleAssign = React.useCallback(
    async (
      pubkey: string,
      assigneeLabel: string,
      options?: { asManagedOwner?: boolean },
    ) => {
      if (assignMutation.isPending || assignmentOperationInFlightRef.current)
        return;
      assignmentOperationInFlightRef.current = true;
      try {
        await assignMutation.mutateAsync({
          assignees: [pubkey],
          assigneeLabel,
          issue,
          signerPubkey: operationSigner,
          signAsManagedOwner: options?.asManagedOwner ?? false,
        });
        setPickerOpen(false);
        setAssigneeQuery("");
        toast.success("Task assigned.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to assign task.",
        );
      } finally {
        assignmentOperationInFlightRef.current = false;
      }
    },
    [assignMutation, issue, operationSigner],
  );

  const handleUnassign = React.useCallback(
    async (pubkey: string, assigneeLabel: string) => {
      if (unassignMutation.isPending || assignmentOperationInFlightRef.current)
        return;
      assignmentOperationInFlightRef.current = true;
      try {
        await unassignMutation.mutateAsync({
          assignees: [pubkey],
          assigneeLabel,
          issue,
          signerPubkey: operationSigner,
          signAsManagedOwner,
        });
        toast.success("Task unassigned.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to unassign task.",
        );
      } finally {
        assignmentOperationInFlightRef.current = false;
      }
    },
    [issue, operationSigner, signAsManagedOwner, unassignMutation],
  );

  React.useEffect(() => {
    if (!pickerOpen) setAssigneeQuery("");
  }, [pickerOpen]);

  const canSelfAssign = viewer !== null && !currentAssignees.has(viewer);
  const isSelfAssigned = viewer !== null && currentAssignees.has(viewer);

  if (issue.assignees.length === 0 && !canAssignOthers && !canSelfAssign) {
    return null;
  }
  const displayedAssignees = showAssignees ? issue.assignees : [];

  return (
    <div
      className={cn(
        "min-w-0 text-muted-foreground",
        contextActions
          ? "grid gap-0.5"
          : "flex flex-wrap items-center gap-1.5 text-xs",
      )}
    >
      {displayedAssignees.map((pubkey) => {
        const profile = profileForPubkey(pubkey, profiles);
        const label = labelForPubkey(pubkey, profiles);
        const canUnassign =
          canAssignOthers ||
          (viewer !== null && normalizePubkey(pubkey) === viewer);
        const avatar = (
          <UserAvatar
            accent={profile?.isAgent === true}
            avatarUrl={profile?.avatarUrl ?? null}
            displayName={label}
            size="xs"
          />
        );
        return (
          <Tooltip key={pubkey}>
            <TooltipTrigger asChild>
              {canUnassign ? (
                <button
                  aria-label={`Unassign ${label}`}
                  className="group relative inline-flex rounded-full"
                  data-testid={`${testIdPrefix}-unassign-${normalizePubkey(pubkey)}`}
                  disabled={unassignMutation.isPending}
                  onClick={() => {
                    void handleUnassign(pubkey, label);
                  }}
                  type="button"
                >
                  {avatar}
                  <span className="absolute inset-0 flex items-center justify-center rounded-full bg-background/80 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100">
                    <X className="h-3 w-3 text-foreground" />
                  </span>
                </button>
              ) : (
                <span className="inline-flex">{avatar}</span>
              )}
            </TooltipTrigger>
            <TooltipContent>
              {canUnassign ? `Unassign ${label}` : `${label} — assigned`}
            </TooltipContent>
          </Tooltip>
        );
      })}
      {canSelfAssign && viewer ? (
        <Button
          className={cn(
            contextActions
              ? "h-5 px-1 text-xs text-muted-foreground hover:text-foreground"
              : "h-5 px-0 text-xs text-primary hover:bg-transparent hover:text-primary hover:underline",
            contextActions && PROJECT_CONTEXT_ACTION_BUTTON_CLASS,
          )}
          data-testid={`${testIdPrefix}-self-assign`}
          disabled={assignMutation.isPending || unassignMutation.isPending}
          onClick={() => {
            void handleAssign(viewer, labelForPubkey(viewer, profiles));
          }}
          size="xs"
          type="button"
          variant="ghost"
        >
          {contextActions ? <UserPlus /> : null}
          Assign to me
        </Button>
      ) : null}
      {showSelfAssignmentState && isSelfAssigned ? (
        <Button
          className={cn(
            "h-5 gap-1 px-1 text-xs text-muted-foreground disabled:opacity-100",
            contextActions && PROJECT_CONTEXT_ACTION_BUTTON_CLASS,
          )}
          disabled
          size="xs"
          type="button"
          variant="ghost"
        >
          <Check className="h-3 w-3" />
          Assigned to me
        </Button>
      ) : null}
      {canAssignOthers ? (
        <Dialog onOpenChange={setPickerOpen} open={pickerOpen}>
          <DialogTrigger asChild>
            <Button
              className={cn(
                "h-5 px-1 text-xs text-muted-foreground hover:text-foreground",
                contextActions && PROJECT_CONTEXT_ACTION_BUTTON_CLASS,
              )}
              data-testid={`${testIdPrefix}-assign`}
              disabled={assignMutation.isPending || unassignMutation.isPending}
              size="xs"
              type="button"
              variant="ghost"
            >
              {contextActions ? <Users /> : null}
              Assign
            </Button>
          </DialogTrigger>
          <DialogContent className="max-w-md gap-0 overflow-hidden p-0">
            <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
              <DialogTitle>Assign task</DialogTitle>
              <DialogDescription>
                Choose a person or agent to work on this task.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2 border-b border-border/60 px-6 py-3">
              <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <Input
                autoFocus
                className="h-8 border-0 px-0 text-sm shadow-none focus-visible:ring-0"
                data-testid="project-assignee-search"
                onChange={(event) => setAssigneeQuery(event.target.value)}
                placeholder="Search people and agents"
                value={assigneeQuery}
              />
            </div>
            <div className="max-h-72 min-h-28 overflow-y-auto p-2">
              {userSearchQuery.isLoading ? (
                <p className="px-3 py-4 text-sm text-muted-foreground">
                  Searching…
                </p>
              ) : candidates.length > 0 ? (
                candidates.map((candidate) => {
                  const label = assigneeSearchLabel(candidate);
                  return (
                    <button
                      className="flex w-full min-w-0 items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                      data-testid={`project-assignee-result-${candidate.pubkey}`}
                      disabled={assignMutation.isPending}
                      key={candidate.pubkey}
                      onClick={() => {
                        void handleAssign(candidate.pubkey, label, {
                          asManagedOwner: signAsManagedOwner,
                        });
                      }}
                      type="button"
                    >
                      <UserAvatar
                        accent={candidate.isAgent}
                        avatarUrl={candidate.avatarUrl}
                        displayName={label}
                        size="xs"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium text-foreground">
                          {label}
                        </span>
                        <span className="block truncate text-xs text-muted-foreground">
                          {candidate.isAgent ? "Agent · " : ""}
                          {truncatePubkey(candidate.pubkey)}
                        </span>
                      </span>
                    </button>
                  );
                })
              ) : (
                <p className="px-3 py-4 text-sm text-muted-foreground">
                  No matching people or agents.
                </p>
              )}
            </div>
          </DialogContent>
        </Dialog>
      ) : null}
    </div>
  );
}
