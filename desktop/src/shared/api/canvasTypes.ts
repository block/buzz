export type CanvasResponse = {
  content: string | null;
  eventId: string | null;
  updatedAt: number | null;
  author: string | null;
};

export type CanvasRevision = {
  eventId: string;
  content: string;
  updatedAt: number;
  author: string;
};

export type SetCanvasInput = {
  channelId: string;
  content: string;
};

export type SetCanvasResult = {
  ok: boolean;
  eventId: string;
};
