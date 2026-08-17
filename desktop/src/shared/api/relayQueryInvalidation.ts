const RELAY_QUERY_ROOTS = new Set<string>([
  "archivedIdentities",
  "channel-canvas",
  "channel-messages",
  "channels",
  "contact-list",
  "custom-emoji",
  "custom-emoji-own",
  "forum-posts",
  "forum-thread",
  "global-notes",
  "home-feed",
  "liked-notes",
  "my-notes",
  "myRelayMembership",
  "oaOwner",
  "presence",
  "profile",
  "pulse-note",
  "pulse-reactions",
  "pulse-timeline",
  "relay-agents",
  "relayMembers",
  "reminders",
  "run-approvals",
  "search-messages",
  "thread-replies",
  "user-profile",
  "user-search",
  "user-status",
  "users-batch",
  "workflow",
  "workflow-runs",
  "workflows",
  "workflows-all",
]);

const RELAY_PROJECT_QUERY_PARTS = new Set<string>([
  "activity-summaries",
  "issues",
  "pull-requests",
]);

const LOCAL_PROJECT_QUERY_PARTS = new Set<string>([
  "commit-diff",
  "local-repo-diff",
  "local-repo-snapshot",
  "local-repositories",
  "repo-diff",
  "repo-snapshot",
  "repo-state",
  "repo-sync-status",
]);

function isRelayDependentProjectQueryKey(queryKey: readonly unknown[]) {
  if (queryKey[0] === "projects") {
    const scope = queryKey[1];
    if (scope === undefined) return true;
    if (typeof scope !== "string") return false;
    if (LOCAL_PROJECT_QUERY_PARTS.has(scope)) return false;
    return RELAY_PROJECT_QUERY_PARTS.has(scope);
  }

  if (queryKey[0] === "project") {
    const scope = queryKey[2];
    if (scope === undefined) return true;
    if (typeof scope !== "string") return false;
    if (LOCAL_PROJECT_QUERY_PARTS.has(scope)) return false;
    return RELAY_PROJECT_QUERY_PARTS.has(scope);
  }

  return false;
}

export function isRelayDependentQueryKey(queryKey: readonly unknown[]) {
  const root = queryKey[0];
  if (typeof root !== "string") return false;
  if (RELAY_QUERY_ROOTS.has(root)) return true;
  return isRelayDependentProjectQueryKey(queryKey);
}

export function isRelayDependentQuery(query: { queryKey: readonly unknown[] }) {
  return isRelayDependentQueryKey(query.queryKey);
}
