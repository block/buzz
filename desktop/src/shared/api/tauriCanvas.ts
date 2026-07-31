import type {
  CanvasRevision,
  CanvasResponse,
  SetCanvasInput,
  SetCanvasResult,
} from "./canvasTypes";
import { invokeTauri } from "./tauri";

type RawCanvasResponse = {
  content: string | null;
  event_id: string | null;
  updated_at: number | null;
  author: string | null;
};

type RawCanvasHistoryResponse = {
  revisions: Array<{
    event_id: string;
    content: string;
    updated_at: number;
    author: string;
  }>;
};

type RawSetCanvasResult = {
  ok: boolean;
  event_id: string;
};

export async function getCanvas(channelId: string): Promise<CanvasResponse> {
  const response = await invokeTauri<RawCanvasResponse>("get_canvas", {
    channelId,
  });
  return {
    content: response.content,
    eventId: response.event_id ?? null,
    updatedAt: response.updated_at ?? null,
    author: response.author ?? null,
  };
}

export async function getCanvasHistory(
  channelId: string,
): Promise<CanvasRevision[]> {
  const response = await invokeTauri<RawCanvasHistoryResponse>(
    "get_canvas_history",
    { channelId },
  );
  return response.revisions.map((revision) => ({
    eventId: revision.event_id,
    content: revision.content,
    updatedAt: revision.updated_at,
    author: revision.author,
  }));
}

export async function setCanvas(
  input: SetCanvasInput,
): Promise<SetCanvasResult> {
  const response = await invokeTauri<RawSetCanvasResult>("set_canvas", {
    channelId: input.channelId,
    content: input.content,
  });
  return {
    ok: response.ok,
    eventId: response.event_id,
  };
}
