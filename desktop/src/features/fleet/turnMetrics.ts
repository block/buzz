/**
 * Pure aggregation for NIP-AM agent turn metrics (kind 44200).
 *
 * One decrypted payload per completed agent turn (see
 * `crates/buzz-core/src/agent_turn_metric.rs` and `docs/nips/NIP-AM.md`).
 * This module folds a bag of payloads into per-agent usage totals for the
 * fleet board. It is deliberately free of React/Tauri imports so the
 * aggregation semantics are unit-testable in isolation.
 *
 * Aggregation semantics (documented so reviewers can check them once):
 *
 * - Payloads are grouped by `sessionId`. Sessionless payloads each form their
 *   own group (they cannot participate in cumulative reconciliation).
 * - Within a session that reported `cumulative` counts, the cumulative from
 *   the highest `turnSeq` wins — it already includes every prior turn of that
 *   session, so summing per-turn deltas on top would double-count.
 * - Sessions without any cumulative sum their per-turn counts. Deltas flagged
 *   `deltaReliable: false` are skipped (NIP-AM: the delta is untrustworthy
 *   after a harness restart mid-session).
 * - Every token field is nullable on the wire — `null`/absent means "not
 *   reported", never zero. Totals stay `null` until at least one payload
 *   reports the field, so the UI can render "—" instead of a misleading 0.
 */

export type TurnMetricTokenCounts = {
  inputTokens?: number | null;
  outputTokens?: number | null;
  totalTokens?: number | null;
  costUsd?: number | null;
};

/** Decrypted kind 44200 payload (camelCase wire shape, unknown fields ignored). */
export type TurnMetricPayload = {
  harness: string;
  timestamp: string;
  model?: string | null;
  channelId?: string | null;
  sessionId?: string | null;
  turnId?: string | null;
  turnSeq?: number | null;
  turn?: TurnMetricTokenCounts | null;
  cumulative?: TurnMetricTokenCounts | null;
  deltaReliable?: boolean;
  stopReason?: string | null;
};

export type AgentUsageTotals = {
  /** Number of metric events folded in (≈ completed turns). */
  turnCount: number;
  /** Sums of reported fields; `null` when no payload ever reported the field. */
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  costUsd: number | null;
  /** Desktop-clock ms of the newest turn timestamp, or null if unparseable. */
  lastTurnAt: number | null;
};

export const EMPTY_USAGE_TOTALS: AgentUsageTotals = {
  turnCount: 0,
  inputTokens: null,
  outputTokens: null,
  totalTokens: null,
  costUsd: null,
  lastTurnAt: null,
};

function isTokenCounts(value: unknown): value is TurnMetricTokenCounts {
  if (typeof value !== "object" || value === null) return false;
  const counts = value as Record<string, unknown>;
  return ["inputTokens", "outputTokens", "totalTokens", "costUsd"].every(
    (field) => {
      const v = counts[field];
      return v === undefined || v === null || typeof v === "number";
    },
  );
}

/**
 * Runtime guard for decrypted payloads. Mirrors NIP-AM's requirements:
 * `harness` and `timestamp` are required, everything else optional. Unknown
 * fields are ignored (forward compatibility). Negative or non-finite costs
 * are rejected the way `AgentTurnMetricPayload::validate` rejects them.
 */
export function isTurnMetricPayload(
  value: unknown,
): value is TurnMetricPayload {
  if (typeof value !== "object" || value === null) return false;
  const payload = value as Record<string, unknown>;
  if (typeof payload.harness !== "string" || payload.harness.length === 0) {
    return false;
  }
  if (typeof payload.timestamp !== "string") return false;
  for (const field of ["turn", "cumulative"]) {
    const counts = payload[field];
    if (counts === undefined || counts === null) continue;
    if (!isTokenCounts(counts)) return false;
    const cost = (counts as TurnMetricTokenCounts).costUsd;
    if (typeof cost === "number" && (!Number.isFinite(cost) || cost < 0)) {
      return false;
    }
  }
  return true;
}

type NullableSums = {
  inputTokens: number | null;
  outputTokens: number | null;
  totalTokens: number | null;
  costUsd: number | null;
};

const NULL_SUMS: NullableSums = {
  inputTokens: null,
  outputTokens: null,
  totalTokens: null,
  costUsd: null,
};

function addCounts(
  sums: NullableSums,
  counts: TurnMetricTokenCounts | null | undefined,
): NullableSums {
  if (!counts) return sums;
  const add = (base: number | null, value: number | null | undefined) => {
    if (typeof value !== "number" || !Number.isFinite(value)) return base;
    return (base ?? 0) + value;
  };
  return {
    inputTokens: add(sums.inputTokens, counts.inputTokens),
    outputTokens: add(sums.outputTokens, counts.outputTokens),
    totalTokens: add(sums.totalTokens, counts.totalTokens),
    costUsd: add(sums.costUsd, counts.costUsd),
  };
}

type SessionAccumulator = {
  turnSums: NullableSums;
  cumulative: TurnMetricTokenCounts | null;
  cumulativeSeq: number;
};

/** Fold a bag of decrypted payloads for ONE agent into usage totals. */
export function aggregateAgentTurnMetrics(
  payloads: readonly TurnMetricPayload[],
): AgentUsageTotals {
  if (payloads.length === 0) return EMPTY_USAGE_TOTALS;

  const sessions = new Map<string, SessionAccumulator>();
  let sessionlessKey = 0;
  let lastTurnAt: number | null = null;

  for (const payload of payloads) {
    const at = Date.parse(payload.timestamp);
    if (Number.isFinite(at) && (lastTurnAt === null || at > lastTurnAt)) {
      lastTurnAt = at;
    }

    // Sessionless payloads cannot be reconciled against a cumulative series;
    // give each its own group so its per-turn counts sum independently.
    const key = payload.sessionId ?? `sessionless:${sessionlessKey++}`;
    let session = sessions.get(key);
    if (!session) {
      session = { turnSums: NULL_SUMS, cumulative: null, cumulativeSeq: -1 };
      sessions.set(key, session);
    }

    if (payload.deltaReliable !== false) {
      session.turnSums = addCounts(session.turnSums, payload.turn);
    }
    if (payload.cumulative) {
      const seq = typeof payload.turnSeq === "number" ? payload.turnSeq : 0;
      if (seq >= session.cumulativeSeq) {
        session.cumulative = payload.cumulative;
        session.cumulativeSeq = seq;
      }
    }
  }

  let totals = NULL_SUMS;
  for (const session of sessions.values()) {
    // Cumulative-at-latest-turn already covers every turn of the session;
    // per-turn sums are the fallback when the session never reported one.
    totals = session.cumulative
      ? addCounts(totals, session.cumulative)
      : addCounts(totals, session.turnSums);
  }

  return {
    turnCount: payloads.length,
    inputTokens: totals.inputTokens,
    outputTokens: totals.outputTokens,
    totalTokens: totals.totalTokens,
    costUsd: totals.costUsd,
    lastTurnAt,
  };
}

/**
 * Token total to display for an agent: provider-reported totals win; when a
 * provider never reported one, fall back to input + output when both exist.
 */
export function displayTokenTotal(totals: AgentUsageTotals): number | null {
  if (totals.totalTokens !== null) return totals.totalTokens;
  if (totals.inputTokens !== null || totals.outputTokens !== null) {
    return (totals.inputTokens ?? 0) + (totals.outputTokens ?? 0);
  }
  return null;
}
