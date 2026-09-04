import * as React from "react";

import type {
  ProjectIssue,
  ProjectPullRequest,
  Repository,
} from "@/features/projects/hooks";
import {
  projectContributorActivityCounts,
  signedProjectContributorPubkeys,
  type ViewerGitIdentity,
} from "@/features/projects/lib/projectContributorMatching";
import { useGitIdentityQuery } from "@/features/projects/useGitIdentity";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { mergeCurrentProfileIntoLookup } from "@/features/profile/lib/identity";
import { useIdentityQuery } from "@/shared/api/hooks";
import { projectPeople } from "./projectDetailHelpers";

/** Resolves profiles and contributor identity data used across project panels. */
export function useProjectDetailPeople({
  issues,
  pullRequests,
  repository,
}: {
  issues: ProjectIssue[];
  pullRequests: ProjectPullRequest[];
  repository: Repository | null;
}) {
  const peoplePubkeys = React.useMemo(() => {
    if (!repository) return [];
    const pullRequestPubkeys = pullRequests.flatMap((pullRequest) => [
      pullRequest.author,
      ...pullRequest.updates.map((update) => update.author),
      ...pullRequest.comments.map((comment) => comment.author),
      ...pullRequest.reviewers,
      ...pullRequest.approvals.map((approval) => approval.author),
    ]);
    const issuePubkeys = issues.flatMap((issue) => [
      issue.author,
      ...issue.recipients,
      ...issue.assignees,
      ...issue.comments.map((comment) => comment.author),
    ]);
    return [
      ...new Set([
        ...projectPeople(repository),
        ...pullRequestPubkeys,
        ...issuePubkeys,
      ]),
    ];
  }, [issues, pullRequests, repository]);
  const profilesQuery = useUsersBatchQuery(peoplePubkeys, {
    enabled: peoplePubkeys.length > 0,
  });
  const currentProfileQuery = useProfileQuery();
  const profiles = React.useMemo(
    () =>
      mergeCurrentProfileIntoLookup(
        profilesQuery.data?.profiles,
        currentProfileQuery.data,
      ),
    [currentProfileQuery.data, profilesQuery.data?.profiles],
  );
  const contributorPubkeys = React.useMemo(() => {
    if (!repository) return [];
    return signedProjectContributorPubkeys({
      contributors: repository.contributors,
      issues,
      owner: repository.owner,
      pullRequests,
    });
  }, [issues, pullRequests, repository]);
  const contributorActivityCounts = React.useMemo(() => {
    if (!repository) return {};
    return projectContributorActivityCounts({
      contributors: repository.contributors,
      issues,
      owner: repository.owner,
      pullRequests,
    });
  }, [issues, pullRequests, repository]);
  const identityQuery = useIdentityQuery();
  const gitIdentityQuery = useGitIdentityQuery();
  const viewerGitIdentity = React.useMemo<ViewerGitIdentity | null>(() => {
    const pubkey = identityQuery.data?.pubkey ?? null;
    if (!pubkey || !gitIdentityQuery.data) return null;
    return {
      pubkey,
      name: gitIdentityQuery.data.name,
      email: gitIdentityQuery.data.email,
    };
  }, [gitIdentityQuery.data, identityQuery.data?.pubkey]);

  return {
    contributorActivityCounts,
    contributorPubkeys,
    identityPubkey: identityQuery.data?.pubkey,
    profiles,
    viewerGitIdentity,
  };
}
