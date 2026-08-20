/**
 * The fibre engine is an external service, not the Buzz relay. The webview
 * reaches it with plain `fetch`; the Tauri CSP already allows `http:`/`https:`
 * origins, so no capability or config change is involved.
 */
const BASE_URL = (
  import.meta.env.VITE_TRIAGE_API_URL ?? "http://localhost:8787"
).replace(/\/$/, "");

export const FIBRE_KINDS = [
  "blocker",
  "decision",
  "ask",
  "commitment",
  "idea",
  "question",
  "fyi",
] as const;

export type FibreKind = (typeof FIBRE_KINDS)[number];

export type FibreStatus = "open" | "done" | "dismissed";

export type FibreSignal = {
  weight: string;
  label: string;
};

export type FibrePerson = {
  pubkey: string;
  label: string;
};

export type FibreArtifact = {
  eventId: string;
  channelId: string | null;
  channelName: string | null;
  threadRootId: string | null;
  authorPubkey: string | null;
  authorLabel: string | null;
  content: string;
  createdAt: number | null;
  isDm?: boolean;
};

export type Fibre = {
  id: string;
  kind: FibreKind;
  status: FibreStatus;
  score: number;
  title: string;
  summary: string;
  why: string;
  whyShort: string;
  signals: FibreSignal[];
  channelId: string | null;
  channelName: string | null;
  isDm: boolean;
  people: FibrePerson[];
  artifacts: FibreArtifact[];
  createdAt: number;
  updatedAt: number;
};

export type FibreIngestMessage = {
  eventId: string;
  channelId: string | null;
  channelName: string | null;
  channelType: string | null;
  authorPubkey: string;
  authorLabel: string;
  createdAt: number;
  content: string;
  threadRootId: string | null;
  isMention: boolean;
  isDm: boolean;
  isReply: boolean;
  isSelf?: boolean;
  source?: "inbox" | "channel" | "live";
};

export type FibresResponse = {
  fibres: Fibre[];
  openCount: number;
  clearedCount: number;
  changes?: unknown[];
  ingested?: number;
};

export type FibreFeedbackAction = "done" | "dismissed" | "delegated";

export type FibreFeedback = {
  pubkey: string;
  fibreId: string;
  eventId?: string;
  channelId?: string | null;
  authorPubkey?: string | null;
  threadRootId?: string | null;
  userAction: FibreFeedbackAction;
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

export function ingestMessages(input: {
  pubkey: string;
  messages: FibreIngestMessage[];
}): Promise<FibresResponse> {
  return request("/ingest", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export function fetchFibres(pubkey: string): Promise<FibresResponse> {
  return request(`/fibres?pubkey=${encodeURIComponent(pubkey)}`);
}

export function patchFibre(input: {
  id: string;
  pubkey: string;
  status: FibreStatus;
}): Promise<FibresResponse & { fibre: Fibre }> {
  return request(`/fibres/${encodeURIComponent(input.id)}`, {
    method: "PATCH",
    body: JSON.stringify({ pubkey: input.pubkey, status: input.status }),
  });
}

export function restoreFibres(pubkey: string): Promise<FibresResponse> {
  return request("/fibres/restore", {
    method: "POST",
    body: JSON.stringify({ pubkey }),
  });
}

export function sendFeedback(input: FibreFeedback): Promise<unknown> {
  return request("/feedback", {
    method: "POST",
    body: JSON.stringify(input),
  });
}
