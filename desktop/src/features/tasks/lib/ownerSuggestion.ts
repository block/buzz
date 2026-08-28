/**
 * REG-16: suggested owners for an UNASSIGNED channel task.
 *
 * Pure — no I/O, no React, no Tauri. Consumed by `ui/SuggestedOwners.tsx`.
 *
 * Design (work/REG-16/reflecting.md §3):
 *   - Calls upstream's GENERIC `rankMentionCandidates` UNMODIFIED to inherit
 *     its member/persona/agent tiering. Fire #41 proved by execution that an
 *     empty query keeps every candidate (every string startsWith ""), so the
 *     comparator degenerates to `groupRank || order` — a stable base tier.
 *   - That base contributes NO owner-specific signal (recorded honestly as K3),
 *     so REG-16 layers its OWN signals on top and must demonstrably reorder.
 *
 * Reason codes are a CLOSED enum with typed params so a future #6588-style
 * nudge agent can consume them instead of re-deriving actionability.
 */
import {
  type MentionCandidateForRanking,
  rankMentionCandidates,
} from "@/features/messages/lib/mentionRanking";
import type { ChannelTask } from "./channelTasks";

/** Closed set — never render a free-text reason. */
export type OwnerSuggestionReasonCode =
  | "mentioned-in-task"
  | "channel-member"
  | "recent-participant"
  | "agent-capability"
  | "task-author"
  | "light-workload";

export type OwnerSuggestionReason = {
  code: OwnerSuggestionReasonCode;
  /** Typed params, not a prebaked sentence — the UI owns wording. */
  params?: Record<string, string | number>;
};

export type OwnerCandidate = MentionCandidateForRanking & {
  pubkey: string;
};

export type OwnerSuggestionContext = {
  /** Pubkeys that have posted in this channel recently, most-recent first. */
  recentParticipantPubkeys?: readonly string[];
  /** Open-task count per pubkey, for the workload signal. */
  openTaskCountByPubkey?: ReadonlyMap<string, number>;
  /** Personas currently runnable, forwarded verbatim to upstream ranking. */
  activePersonaIds?: ReadonlySet<string>;
  /** Max suggestions returned. */
  limit?: number;
};

export type RankedOwnerSuggestion = {
  candidate: OwnerCandidate;
  pubkey: string;
  label: string;
  /** Lower is better. Base tier from upstream, then REG-16's own signals. */
  groupRank: number;
  signalScore: number;
  reasons: OwnerSuggestionReason[];
};

const DEFAULT_LIMIT = 3;
const RECENT_PARTICIPANT_WINDOW = 10;

/** Weights are negative = better. Kept in one table so tests can reason. */
const WEIGHT = {
  mentionedInTask: -6,
  recentParticipant: -3,
  agentCapability: -1,
  lightWorkload: -1,
  /** The author is usually delegating AWAY from themselves — demote. */
  taskAuthor: 4,
  /** Per open task already held. */
  workloadPerTask: 1,
} as const;

/**
 * Does the task text name this candidate? Matches the display name as a whole
 * word, case-insensitively, plus a raw pubkey mention.
 */
function taskMentions(task: ChannelTask, candidate: OwnerCandidate): boolean {
  const haystack = task.title.toLowerCase();
  if (haystack.includes(candidate.pubkey.toLowerCase())) {
    return true;
  }
  const name = candidate.displayName?.trim().toLowerCase();
  if (!name) {
    return false;
  }
  // Whole-word match so "al" never matches "Alice".
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`(^|[^a-z0-9])${escaped}([^a-z0-9]|$)`, "i").test(haystack);
}

/**
 * Rank suggested owners for an unassigned task.
 *
 * Returns `[]` for an already-assigned task (FM1: the affordance must not
 * render at all) and for an empty candidate set.
 */
export function suggestTaskOwners(
  task: ChannelTask,
  candidates: readonly OwnerCandidate[],
  ctx: OwnerSuggestionContext = {},
): RankedOwnerSuggestion[] {
  // FM1 / FM2: a suggestion only exists for a genuinely unassigned task.
  if (task.assignee !== null) {
    return [];
  }
  if (candidates.length === 0) {
    return [];
  }

  // Base tier: upstream's generic ranker, called UNMODIFIED with an empty
  // query. Contributes groupRank + a stable insertion order.
  const base = rankMentionCandidates(
    candidates,
    "",
    ctx.activePersonaIds ?? new Set<string>(),
  );

  const recent = (ctx.recentParticipantPubkeys ?? []).slice(
    0,
    RECENT_PARTICIPANT_WINDOW,
  );
  const workload = ctx.openTaskCountByPubkey ?? new Map<string, number>();

  const scored = base.map((entry, order) => {
    const candidate = entry.candidate;
    const reasons: OwnerSuggestionReason[] = [];
    let signalScore = 0;

    if (taskMentions(task, candidate)) {
      signalScore += WEIGHT.mentionedInTask;
      reasons.push({ code: "mentioned-in-task" });
    }

    if (candidate.isMember) {
      reasons.push({ code: "channel-member" });
    }

    const recentIndex = recent.indexOf(candidate.pubkey);
    if (recentIndex !== -1) {
      // Earlier in the list = more recent = stronger.
      signalScore += WEIGHT.recentParticipant + recentIndex * 0.1;
      reasons.push({
        code: "recent-participant",
        params: { rank: recentIndex + 1 },
      });
    }

    if (candidate.isAgent && candidate.isActiveAgent === true) {
      signalScore += WEIGHT.agentCapability;
      reasons.push({ code: "agent-capability" });
    }

    // Author demotion: whoever filed it is usually not the intended owner.
    if (task.createdBy !== null && task.createdBy === candidate.pubkey) {
      signalScore += WEIGHT.taskAuthor;
      reasons.push({ code: "task-author" });
    }

    const open = workload.get(candidate.pubkey) ?? 0;
    if (open > 0) {
      signalScore += open * WEIGHT.workloadPerTask;
    } else if (workload.size > 0) {
      signalScore += WEIGHT.lightWorkload;
      reasons.push({ code: "light-workload" });
    }

    return {
      candidate,
      pubkey: candidate.pubkey,
      label: entry.label,
      groupRank: entry.groupRank,
      signalScore,
      order,
      reasons,
    };
  });

  return scored
    .sort(
      (a, b) =>
        // REG-16's own signals lead; upstream tier breaks ties; then stable.
        a.signalScore - b.signalScore ||
        a.groupRank - b.groupRank ||
        a.order - b.order,
    )
    .slice(0, ctx.limit ?? DEFAULT_LIMIT)
    .map(({ order: _order, ...rest }) => rest);
}

/** Bare upstream tiering, no REG-16 signals — the K3 comparison baseline. */
export function baselineOwnerOrder(
  candidates: readonly OwnerCandidate[],
  activePersonaIds: ReadonlySet<string> = new Set(),
): string[] {
  return rankMentionCandidates(candidates, "", activePersonaIds).map(
    (entry) => entry.candidate.pubkey,
  );
}
