/**
 * The fibre engine is an external service, not the Buzz relay. The webview
 * reaches it with plain `fetch`; the Tauri CSP already allows `http:`/`https:`
 * origins, so no capability or config change is involved.
 */
const BASE_URL = (
  import.meta.env?.VITE_TRIAGE_API_URL ?? "http://localhost:8787"
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

/**
 * Which column an open fibre lands in. Assigned by the engine in order:
 * priority above the important threshold first, then engagement, then the
 * rest. Every open fibre has exactly one.
 */
export const FIBRE_LANES = ["important", "hot", "other"] as const;

export type FibreLane = (typeof FIBRE_LANES)[number];

export function isFibreLane(value: unknown): value is FibreLane {
  return FIBRE_LANES.includes(value as FibreLane);
}

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
  /** How busy the conversation is, independent of whether it needs you. */
  engagement: number;
  lane: FibreLane;
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

export type FibreLaneCounts = Record<FibreLane, number>;

export type FibresResponse = {
  fibres: Fibre[];
  done: Fibre[];
  openCount: number;
  doneCount: number;
  clearedCount: number;
  laneCounts: FibreLaneCounts;
  changes?: unknown[];
  ingested?: number;
};

export function emptyLaneCounts(): FibreLaneCounts {
  return { important: 0, hot: 0, other: 0 };
}

export function emptyFibresResponse(
  extras?: Partial<FibresResponse>,
): FibresResponse {
  return {
    fibres: [],
    done: [],
    openCount: 0,
    doneCount: 0,
    clearedCount: 0,
    laneCounts: emptyLaneCounts(),
    ingested: 0,
    ...extras,
  };
}

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
  /** Free text from the user on why this was not worth their attention. */
  note?: string;
};

export type FibreFeedbackReceipt = { id: string };

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

export async function ingestMessages(input: {
  pubkey: string;
  messages: FibreIngestMessage[];
}): Promise<Partial<FibresResponse>> {
  return request<Partial<FibresResponse>>("/ingest", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export async function fetchFibres(
  pubkey: string,
): Promise<Partial<FibresResponse>> {
  return request<Partial<FibresResponse>>(
    `/fibres?pubkey=${encodeURIComponent(pubkey)}`,
  );
}

export async function patchFibre(input: {
  id: string;
  pubkey: string;
  status: FibreStatus;
}): Promise<Partial<FibresResponse> & { fibre: Fibre }> {
  return request<Partial<FibresResponse> & { fibre: Fibre }>(
    `/fibres/${encodeURIComponent(input.id)}`,
    {
      method: "PATCH",
      body: JSON.stringify({ pubkey: input.pubkey, status: input.status }),
    },
  );
}

export async function restoreFibres(
  pubkey: string,
): Promise<Partial<FibresResponse>> {
  return request<Partial<FibresResponse>>("/fibres/restore", {
    method: "POST",
    body: JSON.stringify({ pubkey }),
  });
}

export async function sendFeedback(
  input: FibreFeedback,
): Promise<FibreFeedbackReceipt> {
  const { feedback } = await request<{ feedback: FibreFeedbackReceipt }>(
    "/feedback",
    { method: "POST", body: JSON.stringify(input) },
  );
  return feedback;
}

/** Attaches a reason to feedback already recorded, so `x` stays instant. */
export function annotateFeedback(input: {
  pubkey: string;
  feedbackId: string;
  note: string;
}): Promise<unknown> {
  return request(`/feedback/${encodeURIComponent(input.feedbackId)}`, {
    method: "PATCH",
    body: JSON.stringify({ pubkey: input.pubkey, note: input.note }),
  });
}
