import type {
  DispositionState,
  EffectiveOutcome,
  HistoryWarning,
  InvalidRequestReason,
  UnsupportedRequestReason,
} from "@/shared/lib/disposition";

export type TimelineReaction = {
  emoji: string;
  /** Custom (image) emoji URL from the reaction's NIP-30 `emoji` tag, if any. */
  emojiUrl?: string;
  count: number;
  reactedByCurrentUser?: boolean;
  users: Array<{
    pubkey: string;
    displayName: string;
    avatarUrl: string | null;
  }>;
};

/**
 * A marked request's NIP-AD status, straight from the shared verifier
 * (`@/shared/lib/disposition`). See docs/nips/NIP-AD.md.
 *
 * Attached to **every** marked request, including the ones that are not valid
 * obligations. An earlier version attached nothing to those, so they rendered
 * no badge — and the summary chip, counting marked messages without a
 * disposition, reported them as unanswered. That inverted the NIP's own rule
 * that a client fault must never be filed as an agent's gap, and it was
 * reachable by any channel member publishing a malformed marked event.
 *
 * `resolved` is deliberately not `state === "completed"`: an obligation is
 * settled only when a terminal claim bound to it. `responded` and `errored`
 * are non-terminal — the agent answered, or the turn failed, but nothing
 * asserted the work was done.
 */
export type TimelineRequestStatus =
  | {
      kind: "invalid";
      /** A malformed marked event: a client bug, never an agent failure. */
      reason: InvalidRequestReason;
    }
  | {
      kind: "unsupported";
      /** Well-formed, but an intent v1 cannot represent. Nobody's fault. */
      reason: UnsupportedRequestReason;
    }
  | {
      kind: "valid";
      /** Terminal-absorbing outcome. The only basis for a display decision. */
      outcome: EffectiveOutcome;
      /**
       * What arrived last, regardless of absorption. Kept for audit; never
       * use it to decide whether an obligation is done.
       */
      latestObservation: DispositionState | null;
      reason: string;
      /** Diagnostics that do NOT change the outcome — see `HistoryWarning`. */
      warnings: HistoryWarning[];
      /** Settled: the only "done" condition. */
      resolved: boolean;
    };

export type TimelineMessage = {
  id: string;
  /** Stable local key used to avoid remounting optimistic rows on send ack. */
  renderKey?: string;
  createdAt: number;
  pubkey?: string;
  /**
   * Raw signer pubkey (`event.pubkey`), normalized to lowercase hex.
   * Distinct from `pubkey`, which may be a delegated author on an event signed
   * by the active relay. Use this field for checks that require the process or
   * user that cryptographically signed the event.
   */
  signerPubkey?: string;
  author: string;
  /** True when the displayed author is known to be an agent. */
  isAgent?: boolean;
  /** Verified owner pubkey for an agent author, when available. */
  ownerPubkey?: string | null;
  /** Viewer-relative owner label (for example, "you" or "baxen"). */
  ownerLabel?: string | null;
  avatarUrl?: string | null;
  role?: string;
  /** For bot messages, the display name of the persona this bot was created from. */
  personaDisplayName?: string;
  /** For bot messages, the respond-to mode (who can interact with this bot). */
  respondTo?: "owner-only" | "allowlist" | "anyone";
  time: string;
  body: string;
  parentId?: string | null;
  rootId?: string | null;
  depth: number;
  accent?: boolean;
  pending?: boolean;
  edited?: boolean;
  highlighted?: boolean;
  kind?: number;
  tags?: string[][];
  reactions?: TimelineReaction[];
  /** Present on every message marked ["t","request"]. See TimelineRequestStatus. */
  requestStatus?: TimelineRequestStatus;
};
