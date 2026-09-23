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
  /**
   * Signer pubkey of the most recent authorized kind-40003 edit, normalized to
   * lowercase hex. Present only when an edit exists. Used by the
   * `PermissionRequestCard` to enforce edit authenticity: only edits signed by
   * the original agent may resolve the card.
   */
  editSignerPubkey?: string;
  /**
   * The message body BEFORE the most recent edit was overlaid, when an edit
   * exists. For a permission-request sentinel this is the original pending
   * payload; `computePermissionRequest` correlates the resolved edit's
   * `requestNonce`/`sessionId`/`turnId` against it so a same-signer agent
   * cannot cross-apply a resolution meant for a different card.
   */
  preEditBody?: string;
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
};
