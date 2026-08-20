import type { TriageCandidate } from "@/features/triage/lib/collectCandidates";

/**
 * The triage backend is an external service, not the Buzz relay. The webview
 * reaches it with plain `fetch`; the Tauri CSP already allows `http:`/`https:`
 * origins, so no capability or config change is involved.
 */
const BASE_URL = (
  import.meta.env.VITE_TRIAGE_API_URL ?? "http://localhost:8787"
).replace(/\/$/, "");

export type TriageVerdict = "attention" | "noise";

/**
 * A verdict plus the snapshot the service stored with it, so the triage view
 * renders straight from `GET /suggestions` without re-collecting candidates.
 */
export type TriageSuggestion = {
  eventId: string;
  channelId: string | null;
  threadRootId: string | null;
  verdict: TriageVerdict;
  reason: string;
  confidence: number;
  /** True when this verdict came from an explicit past correction. */
  learned?: boolean;
  source?: string;
  channelName: string | null;
  authorPubkey: string | null;
  authorLabel: string | null;
  content: string;
  createdAt: number | null;
  isDm?: boolean;
  isMention?: boolean;
  /** Set once this message owns a todo, which keeps it out of Important. */
  adopted?: boolean;
};

export type TriageTodoStatus = "open" | "done" | "dismissed";

export type TriageTodo = {
  id: string;
  eventId: string;
  channelId: string | null;
  channelName: string | null;
  threadRootId: string | null;
  authorLabel: string | null;
  preview: string;
  reason: string;
  status: TriageTodoStatus;
  createdAt: number;
  resolvedAt?: number;
};

export type TriageFeedbackAction =
  | "adopted"
  | "dismissed"
  | "completed"
  | "promoted";

export type TriageFeedback = {
  pubkey: string;
  eventId: string;
  channelId?: string | null;
  authorPubkey?: string | null;
  threadRootId?: string | null;
  suggestedVerdict: TriageVerdict;
  userAction: TriageFeedbackAction;
  preview?: string;
};

export class TriageApiError extends Error {
  readonly status?: number;

  constructor(message: string, options?: { cause?: unknown; status?: number }) {
    super(message, { cause: options?.cause });
    this.name = "TriageApiError";
    this.status = options?.status;
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`${BASE_URL}${path}`, {
      ...init,
      headers: init?.body
        ? { "content-type": "application/json", ...init?.headers }
        : init?.headers,
    });
  } catch (cause) {
    throw new TriageApiError(
      `Cannot reach the triage service at ${BASE_URL}. Is it running?`,
      { cause },
    );
  }

  if (!response.ok) {
    const detail = await response.text().catch(() => "");
    throw new TriageApiError(
      detail || `Triage service returned ${response.status}`,
      { status: response.status },
    );
  }

  return (await response.json()) as T;
}

export function scanCandidates(input: {
  pubkey: string;
  candidates: TriageCandidate[];
}): Promise<{ suggestions: TriageSuggestion[] }> {
  return request("/scan", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function fetchSuggestions(
  pubkey: string,
): Promise<{ suggestions: TriageSuggestion[] }> {
  return request(`/suggestions?pubkey=${encodeURIComponent(pubkey)}`);
}

export function fetchTodos(pubkey: string): Promise<{ todos: TriageTodo[] }> {
  return request(`/todos?pubkey=${encodeURIComponent(pubkey)}`);
}

export function createTodo(input: {
  pubkey: string;
  eventId: string;
  channelId: string | null;
  channelName: string | null;
  threadRootId: string | null;
  authorLabel: string | null;
  preview: string;
  reason: string;
}): Promise<{ todo: TriageTodo }> {
  return request("/todos", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function updateTodo(input: {
  id: string;
  pubkey: string;
  status: TriageTodoStatus;
}): Promise<{ todo: TriageTodo }> {
  return request(`/todos/${encodeURIComponent(input.id)}`, {
    method: "PATCH",
    body: JSON.stringify({ pubkey: input.pubkey, status: input.status }),
  });
}

export function sendFeedback(input: TriageFeedback): Promise<unknown> {
  return request("/feedback", {
    method: "POST",
    body: JSON.stringify(input),
  });
}
