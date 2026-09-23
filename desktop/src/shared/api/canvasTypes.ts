export type CanvasResponse = {
  content: string | null;
  eventId: string | null;
  updatedAt: number | null;
  author: string | null;
};

export type SetCanvasInput = {
  channelId: string;
  content: string;
  expectedRevision?: string | null;
};

export type SetCanvasResult = {
  ok: boolean;
  eventId: string;
  /**
   * `false` when the write was accepted by the relay but the post-write
   * verification read failed, so supersession could not be checked. The save is
   * durable; the caller shows a non-destructive "saved, verification
   * unavailable" note rather than a failure. `true` on the normal verified path.
   */
  verified: boolean;
};

export type CanvasRevision = {
  eventId: string;
  content: string;
  createdAt: number;
  author: string;
};

/** Composite `(created_at DESC, id ASC)` cursor for "Load older" paging. */
export type CanvasHistoryCursor = {
  createdAt: number;
  eventId: string;
};

export type CanvasHistoryResponse = {
  revisions: CanvasRevision[];
  /** Present only when older revisions may remain. */
  nextCursor: CanvasHistoryCursor | null;
};
