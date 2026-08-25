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
