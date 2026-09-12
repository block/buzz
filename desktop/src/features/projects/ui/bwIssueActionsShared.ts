import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import type { Repository as Project } from "@/features/projects/hooks";
import { bwChainHead, type BwSnapshot } from "@/features/projects/bwProjection";

/** Shared by every BW write panel (`BwIssueActions.tsx`,
 * `BwAssignmentSection.tsx`): refresh both the per-project issue list and
 * the cross-project work-items view after a write succeeds. */
export function useInvalidateProjectIssues(project: Project) {
  const queryClient = useQueryClient();
  return React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: ["project", project.id, "issues"],
      }),
      queryClient.invalidateQueries({ queryKey: ["projects", "work-items"] }),
    ]);
  }, [project.id, queryClient]);
}

export function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error ? error.message : fallback;
}

/** Derives the `BwReadyAction` stream dropdown's options from the issue's
 * repository branches (`useRepoStateQuery`, backed by the repo's owner-
 * signed kind:30618 state — the same remote-branch source `git ls-remote`
 * would report). The dropdown never falls back to free text or a guessed
 * default branch: an empty/unloaded branch list blocks the ready submit
 * outright, since NIP-BW's `stream` field must name a real branch. */
export function bwReadyStreamOptions(
  branches: Array<{ name: string }> | undefined,
  isLoading: boolean,
): { options: string[]; unavailable: boolean } {
  const options = [...new Set((branches ?? []).map((branch) => branch.name))];
  return { options, unavailable: !isLoading && options.length === 0 };
}

/** NIP-BW's `ready` record requires a non-null `update` id naming the
 * issue's current text snapshot (`crates/buzz-core/src/bw/schema.json`'s
 * `issue-state.update: "id"` — not the nullable `"?id"` `assignment`/`rework`
 * get). An issue that has never had its own `issue-update` record has no
 * head to send, so `bwChainHead` reports `null` — submitting that null
 * anyway does not read as "no update" to Core, it fails the required `id`
 * shape check outright (`bw:reject:shape:schema`). Block the ready submit
 * client-side instead of letting that cryptic core-level rejection surface;
 * the fix is to save a text update first (the editor is already shown above
 * this action while backlog), which is exactly the record this needs. */
export function bwReadyUpdateUnavailable(
  snapshot: BwSnapshot,
  issueId: string,
): boolean {
  return bwChainHead(snapshot, issueId, "issue-update").headId === null;
}
