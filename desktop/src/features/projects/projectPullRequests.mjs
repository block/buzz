import { getAllTags, getTag } from "./projectIssues.mjs";

function latestUpdateForPullRequest(pullRequestId, updateEvents) {
  return updateEvents
    .filter((event) => getTag(event, "E") === pullRequestId)
    .sort((left, right) => right.created_at - left.created_at)[0];
}

function getCloneUrls(event) {
  return event.tags
    .filter((tag) => tag[0] === "clone")
    .flatMap((tag) => tag.slice(1))
    .filter(Boolean);
}

export function eventToProjectPullRequest(pullRequest, updateEvents = []) {
  const latestUpdate = latestUpdateForPullRequest(pullRequest.id, updateEvents);
  const title =
    getTag(pullRequest, "subject") ||
    pullRequest.content.split("\n")[0] ||
    "Untitled pull request";
  const latestCommit = getTag(latestUpdate ?? pullRequest, "c") ?? null;

  return {
    id: pullRequest.id,
    title,
    content: pullRequest.content,
    author: pullRequest.pubkey,
    createdAt: pullRequest.created_at,
    repoAddress: getTag(pullRequest, "a") ?? null,
    labels: getAllTags(pullRequest, "t"),
    recipients: getAllTags(pullRequest, "p"),
    branchName: getTag(pullRequest, "branch-name") ?? null,
    commit: latestCommit,
    cloneUrls: getCloneUrls(latestUpdate ?? pullRequest),
    updateCount: updateEvents.filter(
      (event) => getTag(event, "E") === pullRequest.id,
    ).length,
    updatedAt: latestUpdate?.created_at ?? pullRequest.created_at,
  };
}

export function projectPullRequestEventsToPullRequests(
  pullRequestEvents,
  updateEvents = [],
) {
  return [...pullRequestEvents]
    .map((pullRequest) => eventToProjectPullRequest(pullRequest, updateEvents))
    .sort((left, right) => right.updatedAt - left.updatedAt);
}
