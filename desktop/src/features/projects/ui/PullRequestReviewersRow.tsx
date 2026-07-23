import { Check, Search, UserPlus, X } from "lucide-react";
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
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
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
                  <span className="-right-1 -bottom-1 absolute flex h-3.5 w-3.5 items-center justify-center rounded-full bg-green-600 text-white ring-2 ring-background">
                    <Check className="h-2.5 w-2.5" />
                  </span>
                ) : hasRequestedChanges ? (
                  <span className="-right-1 -bottom-1 absolute flex h-3.5 w-3.5 items-center justify-center rounded-full bg-destructive text-destructive-foreground ring-2 ring-background">
                    <X className="h-2.5 w-2.5" />
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
        <Popover onOpenChange={setPickerOpen} open={pickerOpen}>
          <PopoverTrigger asChild>
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
          </PopoverTrigger>
          <PopoverContent align="start" className="w-72 p-2">
            <div className="flex items-center gap-2 border-b border-border/70 px-2 pb-2">
              <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <Input
                autoFocus
                className="h-7 border-0 px-0 text-xs shadow-none focus-visible:ring-0"
                data-testid="project-reviewer-search"
                onChange={(event) => setReviewerQuery(event.target.value)}
                placeholder="Search people and agents"
                value={reviewerQuery}
              />
            </div>
            <div className="mt-2 max-h-64 overflow-y-auto">
              {userSearchQuery.isLoading ? (
                <p className="px-2 py-2 text-xs text-muted-foreground">
                  Searching…
                </p>
              ) : candidates.length > 0 ? (
                candidates.map((candidate) => {
                  const label = reviewerSearchLabel(candidate);
                  return (
                    <button
                      className="flex w-full min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-left hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
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
                        <span className="block truncate text-xs font-medium text-foreground">
                          {label}
                        </span>
                        <span className="block truncate text-2xs text-muted-foreground">
                          {candidate.isAgent ? "Agent · " : ""}
                          {truncatePubkey(candidate.pubkey)}
                        </span>
                      </span>
                    </button>
                  );
                })
              ) : (
                <p className="px-2 py-2 text-xs text-muted-foreground">
                  No matching people or agents.
                </p>
              )}
            </div>
          </PopoverContent>
        </Popover>
      ) : null}
    </div>
  );
}
