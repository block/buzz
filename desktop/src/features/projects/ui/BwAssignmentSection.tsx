import * as React from "react";
import { toast } from "sonner";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import {
  buildBwAssignmentCandidates,
  filterBwAssignmentCandidates,
  resolveBwAssignmentSelection,
} from "@/features/projects/bwAssignmentCandidates";
import { bwAssignmentHead } from "@/features/projects/bwProjection";
import type {
  ProjectIssue,
  Repository as Project,
} from "@/features/projects/hooks";
import {
  BwConflictError,
  submitBwAssignment,
} from "@/features/projects/bwWrite";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { MentionAutocomplete } from "@/features/messages/ui/MentionAutocomplete";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  errorMessage,
  useInvalidateProjectIssues,
} from "./bwIssueActionsShared";

/** Select or release the sole delegate for a `backlog`/`ready` issue, over
 * the *existing* kind:1 assignment wire (NIP-BW.md: "no new assignment
 * grammar"). Unassign always resubmits the exact current writer as `p` —
 * Core's own causality check refuses an unassignment whose `p` does not
 * match the head it chains from, so this never lets the UI invent a
 * mismatched release.
 *
 * The writer is picked the same way the composer's `@`-mention does: type a
 * name, choose a `MentionAutocomplete` suggestion built from the relay agent
 * directory plus the repo-bound channel's members (this panel has no
 * composer `channelId` of its own). Selecting a suggestion only stages the
 * exact pubkey it carries — the wire payload (`submitBwAssignment`'s
 * `delegate`) is unchanged either way. Raw hex stays reachable via a toggle
 * for identities with no profile/agent/member entry to search by name. */
export function BwAssignmentSection({
  issue,
  profiles,
  project,
}: {
  issue: ProjectIssue;
  profiles?: UserProfileLookup;
  project: Project;
}) {
  const [manualEntry, setManualEntry] = React.useState(false);
  const [delegate, setDelegate] = React.useState("");
  const [pickerQuery, setPickerQuery] = React.useState("");
  const [pickerOpen, setPickerOpen] = React.useState(false);
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
  const suggestions = React.useMemo(
    () =>
      filterBwAssignmentCandidates(candidates, pickerQuery).map(
        (candidate) => ({
          avatarUrl: candidate.avatarUrl,
          displayName: candidate.displayName,
          isAgent: candidate.isAgent,
          pubkey: candidate.pubkey,
        }),
      ),
    [candidates, pickerQuery],
  );

  if (!issue.bw) return null;
  const snapshot = issue.bw.snapshot;
  const head = bwAssignmentHead(snapshot, issue.id);

  const reset = () => {
    setDelegate("");
    setPickerQuery("");
    setPickerOpen(false);
    setManualEntry(false);
  };

  const run = async (operation: "assignment" | "unassignment", who: string) => {
    if (pending) return;
    setPending(true);
    try {
      await submitBwAssignment({
        delegate: who,
        issueId: issue.id,
        operation,
        repo: project.repoAddress,
        snapshot,
      });
      toast.success(
        operation === "assignment" ? "Writer assigned." : "Writer unassigned.",
      );
      reset();
      await invalidate();
    } catch (error) {
      toast.error(
        error instanceof BwConflictError
          ? error.message
          : errorMessage(error, "Assignment was refused."),
      );
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

  if (head.writer) {
    const writerProfile = profiles?.[normalizePubkey(head.writer)];
    const writerLabel = resolveUserLabel({ profiles, pubkey: head.writer });
    return (
      <div className="space-y-1.5" data-testid="bw-assignment-section">
        <div className="flex items-center justify-between gap-2 text-xs">
          <span
            className="flex min-w-0 items-center gap-1.5"
            data-testid="bw-assignment-writer"
            title={head.writer}
          >
            <UserAvatar
              accent={writerProfile?.isAgent === true}
              avatarUrl={writerProfile?.avatarUrl ?? null}
              displayName={writerLabel}
              size="xs"
            />
            <span className="min-w-0 truncate font-medium">{writerLabel}</span>
            <span className="shrink-0 font-mono text-muted-foreground">
              {truncatePubkey(head.writer)}
            </span>
          </span>
          <button
            className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
            data-testid="bw-unassign"
            disabled={pending}
            onClick={() => void run("unassignment", head.writer as string)}
            type="button"
          >
            {pending ? "Unassigning…" : "Unassign"}
          </button>
        </div>
      </div>
    );
  }

  const delegateValid = /^[0-9a-f]{64}$/i.test(delegate.trim());

  return (
    <div className="space-y-1.5" data-testid="bw-assignment-section">
      {manualEntry ? (
        <div className="flex items-center gap-2">
          <input
            className="h-8 flex-1 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
            data-testid="bw-assign-delegate"
            onChange={(event) => setDelegate(event.target.value)}
            placeholder="Delegate pubkey"
            value={delegate}
          />
          <button
            className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
            data-testid="bw-assign"
            disabled={pending || !delegateValid}
            onClick={() =>
              void run("assignment", delegate.trim().toLowerCase())
            }
            type="button"
          >
            {pending ? "Assigning…" : "Assign"}
          </button>
        </div>
      ) : (
        <div className="relative flex items-center gap-2">
          <input
            className="h-8 flex-1 rounded-md border border-border/60 bg-background px-2 text-xs text-foreground"
            data-testid="bw-assign-picker-query"
            onBlur={() => {
              // Deferred so a suggestion's onMouseDown (which preventDefaults
              // the blur) still lands before the dropdown closes.
              window.setTimeout(() => setPickerOpen(false), 0);
            }}
            onChange={(event) => {
              setPickerQuery(event.target.value);
              setPickerOpen(true);
              setDelegate("");
            }}
            onFocus={() => setPickerOpen(true)}
            placeholder="Assign writer by name"
            value={pickerQuery}
          />
          <button
            className="rounded-md border border-border/60 px-2.5 py-1 text-xs font-medium disabled:opacity-60"
            data-testid="bw-assign"
            disabled={pending || !delegateValid}
            onClick={() =>
              void run("assignment", delegate.trim().toLowerCase())
            }
            type="button"
          >
            {pending ? "Assigning…" : "Assign"}
          </button>
          {pickerOpen ? (
            <MentionAutocomplete
              onSelect={(suggestion) => {
                const pubkey = resolveBwAssignmentSelection(suggestion);
                if (!pubkey) return;
                setDelegate(pubkey);
                setPickerQuery(suggestion.displayName);
                setPickerOpen(false);
              }}
              position="below"
              selectedIndex={0}
              suggestions={suggestions}
            />
          ) : null}
        </div>
      )}
      <button
        className="text-2xs text-muted-foreground underline-offset-2 hover:underline"
        data-testid="bw-assign-toggle-manual"
        onClick={() => {
          setManualEntry((current) => !current);
          setDelegate("");
          setPickerQuery("");
          setPickerOpen(false);
        }}
        type="button"
      >
        {manualEntry ? "Search by name instead" : "Enter pubkey manually"}
      </button>
    </div>
  );
}
