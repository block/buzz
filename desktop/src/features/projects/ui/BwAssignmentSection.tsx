import * as React from "react";
import { toast } from "sonner";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { buildBwAssignmentCandidates } from "@/features/projects/bwAssignmentCandidates";
import { bwAssignmentHead } from "@/features/projects/bwProjection";
import type {
  ProjectIssue,
  Repository as Project,
} from "@/features/projects/hooks";
import {
  BwConflictError,
  submitBwWriterSelection,
} from "@/features/projects/bwWrite";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import {
  errorMessage,
  useInvalidateProjectIssues,
} from "./bwIssueActionsShared";

/** Select or replace the sole delegate for a `backlog`/`ready` issue, over
 * the *existing* kind:1 assignment wire (NIP-BW.md: "no new assignment
 * grammar"). The visible control contains resolved names only; pubkeys remain
 * the option values used for the signed wire identity and are never rendered.
 *
 * A ready-state replacement also republishes ready against the newly created
 * assignment head. This keeps the selected writer and Start development's
 * causal binding identical instead of displaying an optimistic local swap. */
export function BwAssignmentSection({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const [pending, setPending] = React.useState(false);
  const invalidate = useInvalidateProjectIssues(project);
  const relayAgentsQuery = useRelayAgentsQuery();
  const membersQuery = useChannelMembersQuery(project.channelId ?? null);

  const candidates = React.useMemo(
    () =>
      buildBwAssignmentCandidates({
        members: membersQuery.data ?? [],
        profiles,
        relayAgents: (relayAgentsQuery.data ?? []).map((agent) => ({
          displayName: agent.name,
          pubkey: agent.pubkey,
        })),
      }),
    [membersQuery.data, profiles, relayAgentsQuery.data],
  );

  if (!issue.bw) return null;
  const snapshot = issue.bw.snapshot;
  const head = bwAssignmentHead(snapshot, issue.id);
  const currentCandidate = head.writer
    ? candidates.find((candidate) => candidate.pubkey === head.writer)
    : null;
  const options =
    head.writer && !currentCandidate
      ? [
          {
            avatarUrl: null,
            displayName: "Current writer",
            isAgent: true,
            pubkey: head.writer,
          },
          ...candidates,
        ]
      : candidates;

  const selectWriter = async (delegate: string) => {
    if (pending || !delegate || delegate === head.writer) return;
    setPending(true);
    try {
      await submitBwWriterSelection({
        delegate,
        issueId: issue.id,
        repo: issue.repoAddress ?? project.repoAddress,
      });
      toast.success(head.writer ? "Writer changed." : "Writer assigned.");
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Writer change was refused."),
      );
      await invalidate();
    } finally {
      setPending(false);
    }
  };

  if (head.conflict) {
    return (
      <p
        className="rounded-md border border-destructive/40 bg-destructive/10 p-2 text-xs text-destructive"
        data-testid="bw-assignment-conflict"
        role="alert"
      >
        Concurrent assignment changes are in conflict. Resolve manually.
      </p>
    );
  }

  return (
    <div className="space-y-1.5" data-testid="bw-assignment-section">
      <select
        aria-label="Selected writer"
        className="h-8 w-full rounded-md border border-border/60 bg-background px-2 text-xs text-foreground outline-hidden focus:ring-1 focus:ring-ring disabled:opacity-60"
        data-testid="bw-assignment-writer"
        disabled={pending || options.length === 0}
        onChange={(event) => void selectWriter(event.target.value)}
        value={head.writer ?? ""}
      >
        {!head.writer ? (
          <option disabled value="">
            {pending ? "Assigning writer…" : "Select writer"}
          </option>
        ) : null}
        {options.map((candidate) => (
          <option key={candidate.pubkey} value={candidate.pubkey}>
            {candidate.displayName}
          </option>
        ))}
      </select>
    </div>
  );
}
