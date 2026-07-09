import assert from "node:assert/strict";
import test from "node:test";

import { isRelayDependentQueryKey } from "./relayQueryInvalidation.ts";

test("relay invalidation includes relay-backed channel and profile queries", () => {
  for (const queryKey of [
    ["channels"],
    ["channels", "channel-1", "members"],
    ["channel-messages", "channel-1"],
    ["thread-replies", "channel-1", "root-1"],
    ["forum-posts", "channel-1"],
    ["home-feed"],
    ["users-batch", "alice"],
    ["presence", "alice"],
    ["user-status", "alice"],
    ["relay-agents"],
    ["relayMembers"],
    ["archivedIdentities"],
    ["oaOwner", "alice"],
  ]) {
    assert.equal(isRelayDependentQueryKey(queryKey), true, queryKey.join("/"));
  }
});

test("relay invalidation includes social/workflow relay queries", () => {
  for (const queryKey of [
    ["global-notes"],
    ["liked-notes", "alice"],
    ["pulse-reactions", "note-1"],
    ["workflows", "channel-1"],
    ["workflows-all", "channel-1"],
    ["workflow-runs", "workflow-1"],
    ["run-approvals", "workflow-1", "run-1"],
    ["reminders", "alice"],
    ["custom-emoji"],
  ]) {
    assert.equal(isRelayDependentQueryKey(queryKey), true, queryKey.join("/"));
  }
});

test("relay invalidation excludes local Tauri and disk-only query roots", () => {
  for (const queryKey of [
    ["identity"],
    ["managed-agents"],
    ["personas"],
    ["teams"],
    ["acp-runtimes"],
    ["backend-providers"],
    ["managed-agent-log", "agent-1", 200],
    ["workspace-icon", "wss://relay.example"],
    ["agent-memory", "agent-1"],
  ]) {
    assert.equal(isRelayDependentQueryKey(queryKey), false, queryKey.join("/"));
  }
});

test("relay invalidation separates relay project queries from local repo work", () => {
  for (const queryKey of [
    ["projects"],
    ["project", "project-1"],
    ["project", "project-1", "issues"],
    ["project", "project-1", "pull-requests"],
    ["projects", "issues", ["project-1"]],
    ["projects", "activity-summaries", ["addr-1"]],
  ]) {
    assert.equal(isRelayDependentQueryKey(queryKey), true, queryKey.join("/"));
  }

  for (const queryKey of [
    ["project", "project-1", "repo-state"],
    ["project", "project-1", "repo-snapshot", "main"],
    ["project", "project-1", "repo-diff", "main"],
    ["project", "project-1", "local-repo-diff"],
    ["project", "project-1", "commit-diff", "remote"],
    ["projects", "local-repositories", "default"],
    ["projects", "repo-snapshots", "default", ["project-1"]],
  ]) {
    assert.equal(isRelayDependentQueryKey(queryKey), false, queryKey.join("/"));
  }
});
