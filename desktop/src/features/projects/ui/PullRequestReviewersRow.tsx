import { Check, Search, TriangleAlert } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useIsArchivedPredicate } from "@/features/identity-archive/hooks";
import type { Project, ProjectPullRequest } from "@/features/projects/hooks";
import { useRequestProjectPullRequestReviewMutation } from "@/features/projects/pullRequestReviews";
import { useUserSearchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { UserSearchResult } from "@/shared/api/types";
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

function reviewerSearchLabel(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

/** Reviewer status avatars and the reviewer request picker for a pull request. */
export function PullRequestReviewersRow({
  canRequest,
  profiles,
  project,
  pullRequest,
  signAsManagedOwner,
}: {
  canRequest: boolean;
  profiles?: UserProfileLookup;
  project: Project;
  pullRequest: ProjectPullRequest;
  signAsManagedOwner: boolean;
}) {
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [reviewerQuery, setReviewerQuery] = React.useState("");
  const requestInFlightRef = React.useRef(false);
  const requestReviewMutation =
    useRequestProjectPullRequestReviewMutation(project);
  const deferredReviewerQuery = React.useDeferredValue(reviewerQuery.trim());
  const requestedReviewers = React.useMemo(
    () => new Set(pullRequest.reviewers.map(normalizePubkey)),
    [pullRequest.reviewers],
  );
  const pullRequestAuthor = normalizePubkey(pullRequest.author);
  const userSearchQuery = useUserSearchQuery(deferredReviewerQuery, {
    allowEmpty: true,
    enabled: canRequest && pickerOpen,
    limit: 50,
  });
  const isArchivedDiscovery = useIsArchivedPredicate();
  const candidates = React.useMemo(
    () =>
      (userSearchQuery.data ?? []).filter((user) => {
        const pubkey = normalizePubkey(user.pubkey);
        return (
          pubkey !== pullRequestAuthor &&
          !requestedReviewers.has(pubkey) &&
          !isArchivedDiscovery(pubkey)
        );
      }),
    [
      isArchivedDiscovery,
      pullRequestAuthor,
      requestedReviewers,
      userSearchQuery.data,
    ],
  );
  const approvedBy = new Set(
    pullRequest.approvals.map((approval) => normalizePubkey(approval.author)),
  );
  const changesRequestedBy = new Set(
    pullRequest.changeRequests.map((request) =>
      normalizePubkey(request.author),
    ),
  );

  const handleRequest = React.useCallback(
    async (pubkey: string, reviewerLabel: string) => {
      if (requestReviewMutation.isPending || requestInFlightRef.current) return;
      requestInFlightRef.current = true;
      try {
        await requestReviewMutation.mutateAsync({
          pullRequest,
          reviewers: [pubkey],
          reviewerLabel,
          signAsManagedOwner,
        });
        setPickerOpen(false);
        setReviewerQuery("");
        toast.success("Review requested.");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to request review.",
        );
      } finally {
        requestInFlightRef.current = false;
      }
    },
    [pullRequest, requestReviewMutation, signAsManagedOwner],
  );

  React.useEffect(() => {
    if (!pickerOpen) setReviewerQuery("");
  }, [pickerOpen]);

  if (pullRequest.reviewers.length === 0 && !canRequest) {
    return null;
  }

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
      {pullRequest.reviewers.map((pubkey) => {
        const profile = profileForPubkey(pubkey, profiles);
        const label = labelForPubkey(pubkey, profiles);
        const hasApproved = approvedBy.has(normalizePubkey(pubkey));
        const hasRequestedChanges = changesRequestedBy.has(
          normalizePubkey(pubkey),
        );
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
                  <span className="-right-0.5 -bottom-0.5 absolute flex h-2.5 w-2.5 items-center justify-center rounded-full bg-green-600 text-white ring-1 ring-background">
                    <Check className="h-1.5 w-1.5" />
                  </span>
                ) : hasRequestedChanges ? (
                  <span className="-right-0.5 -bottom-0.5 absolute flex h-2.5 w-2.5 items-center justify-center rounded-full bg-amber-500 text-amber-950 ring-1 ring-background">
                    <TriangleAlert className="h-1.5 w-1.5" />
                  </span>
                ) : null}
              </span>
            </TooltipTrigger>
            <TooltipContent>
              {label}
              {hasApproved
                ? " — approved"
                : hasRequestedChanges
                  ? " — requested changes"
                  : " — review requested"}
            </TooltipContent>
          </Tooltip>
        );
      })}
      {canRequest ? (
        <Dialog onOpenChange={setPickerOpen} open={pickerOpen}>
          <DialogTrigger asChild>
            <Button
              className="h-6 px-1 text-xs text-muted-foreground hover:text-foreground"
              disabled={requestReviewMutation.isPending}
              size="xs"
              type="button"
              variant="ghost"
            >
              Add Reviewer
            </Button>
          </DialogTrigger>
          <DialogContent className="max-w-md gap-0 overflow-hidden p-0">
            <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
              <DialogTitle>Add reviewer</DialogTitle>
              <DialogDescription>
                Choose a person or agent to review this pull request.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2 border-b border-border/60 px-6 py-3">
              <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <Input
                autoFocus
                className="h-8 border-0 px-0 text-sm shadow-none focus-visible:ring-0"
                data-testid="project-reviewer-search"
                onChange={(event) => setReviewerQuery(event.target.value)}
                placeholder="Search people and agents"
                value={reviewerQuery}
              />
            </div>
            <div className="max-h-72 min-h-28 overflow-y-auto p-2">
              {userSearchQuery.isLoading ? (
                <p className="px-3 py-4 text-sm text-muted-foreground">
                  Searching…
                </p>
              ) : candidates.length > 0 ? (
                candidates.map((candidate) => {
                  const label = reviewerSearchLabel(candidate);
                  return (
                    <button
                      className="flex w-full min-w-0 items-center gap-3 rounded-lg px-3 py-2 text-left hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                      data-testid={`project-reviewer-result-${candidate.pubkey}`}
                      disabled={requestReviewMutation.isPending}
                      key={candidate.pubkey}
                      onClick={() => {
                        void handleRequest(candidate.pubkey, label);
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
