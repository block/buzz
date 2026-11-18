import { invokeTauri } from "@/shared/api/tauri";
import type {
  CanvasHistoryCursor,
  CanvasHistoryResponse,
  CanvasResponse,
  SetCanvasInput,
  SetCanvasResult,
} from "@/shared/api/canvasTypes";

type RawCanvasResponse = {
  content: string | null;
  event_id: string | null;
  updated_at: number | null;
  author: string | null;
};

type RawCanvasHistoryResponse = {
  revisions: {
    event_id: string;
    content: string;
    created_at: number;
    author: string;
  }[];
  next_cursor: { created_at: number; event_id: string } | null;
};

type RawSetCanvasResult = {
  ok: boolean;
  event_id: string;
  verified: boolean;
};

export async function getCanvas(channelId: string): Promise<CanvasResponse> {
  const response = await invokeTauri<RawCanvasResponse>("get_canvas", {
    channelId,
  });
  return {
    content: response.content,
    eventId: response.event_id ?? null,
    // Normalize absent keys to null: ensureWelcomeCanvas treats null as
    // "no canvas yet", and `undefined !== null` would make every fresh
    // channel look already-seeded.
    updatedAt: response.updated_at ?? null,
    author: response.author ?? null,
  };
}

export async function setCanvas(
  input: SetCanvasInput,
): Promise<SetCanvasResult> {
  const response = await invokeTauri<RawSetCanvasResult>("set_canvas", {
    channelId: input.channelId,
    content: input.content,
    expectedRevision: input.expectedRevision ?? null,
  });
  return {
    ok: response.ok,
    eventId: response.event_id,
    verified: response.verified,
  };
}

export async function getCanvasHistory(
  channelId: string,
  options: { limit?: number; cursor?: CanvasHistoryCursor | null } = {},
): Promise<CanvasHistoryResponse> {
  const response = await invokeTauri<RawCanvasHistoryResponse>(
    "get_canvas_history",
    {
      channelId,
      limit: options.limit ?? null,
      until: options.cursor?.createdAt ?? null,
      beforeId: options.cursor?.eventId ?? null,
    },
  );
  return {
    revisions: response.revisions.map((revision) => ({
      eventId: revision.event_id,
      content: revision.content,
      createdAt: revision.created_at,
      author: revision.author,
    })),
    nextCursor: response.next_cursor
      ? {
          createdAt: response.next_cursor.created_at,
          eventId: response.next_cursor.event_id,
        }
      : null,
  };
}
