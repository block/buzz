// Typed client for the daemon's loopback HTTP API (proxied at /api by vite).

export type Selection = {
  channels: string[];
  authors: string[];
  kinds: number[];
};

/** Half-open run window; omitted bounds mean "everything up to now". */
export type TimeWindow = { since?: number; until_exclusive?: number };

export type Status = {
  relay: string;
  pubkey: string;
  started_at: number;
  connection: string;
  connected_at: number | null;
  reconnect_attempts: number;
  last_error: string | null;
  last_event_at: number | null;
  backfill_complete: boolean;
  channels: Record<string, unknown>;
  total_events: number;
  folds: number;
  artifacts: number;
  db_path: string;
};

export type Channel = {
  id: string;
  name: string | null;
  channel_type: string;
  backfill_cursor: number | null;
  backfill_done: boolean;
  active: boolean;
  discovered_at: number;
};

export type Bucket = { day: number; count: number };

export type Preview = {
  selection: Selection;
  window: { since: number; until_exclusive: number };
  count: number;
  total_chars: number;
  oldest_ts: number | null;
  newest_ts: number | null;
  /** Absent when the daemon predates the /select/events micro-batch. */
  buckets?: Bucket[];
};

export type EventItem = {
  id: string;
  channel: string | null;
  pubkey: string;
  author_name: string | null;
  kind: number;
  created_at: number;
  content: string;
};

export type PageCursor = { created_at: number; id: string };

export type EventsPage = {
  window: { since: number; until_exclusive: number };
  count: number;
  next: PageCursor | null;
  events: EventItem[];
};

export type FoldSpec = {
  name: string;
  selection: Selection;
  schema: string;
  model: string;
  instructions: string;
  meta?: unknown;
};

export type FoldRow = {
  name: string;
  spec: FoldSpec;
  versions: number;
  latest_version: number | null;
  created_at: number;
  updated_at: number;
};

export type WindowFit = {
  model_window: number | null;
  est_input_tokens: number;
  fits: boolean | null;
  headroom_tokens: number | null;
};

export type Estimate = { est_input_tokens: number; window_fit: WindowFit };

export type Preflight =
  | { plan: "cached" }
  | { plan: "stalled"; reason: string; pending: number }
  | {
      plan: "ready";
      shown: number;
      pending: number;
      truncated: boolean;
      estimate: Estimate;
      window: [number, number];
    };

export type Artifact = {
  fold: string;
  version: number;
  output: string;
  shown_ids: string[];
  coverage_since: number | null;
  coverage_until: number | null;
  selection: Selection;
  channels: string[];
  model: string;
  schema: string;
  prompt_sha256: string;
  truncated: boolean;
  created_at: number;
};

export type ArtifactSummary = {
  version: number;
  created_at: number;
  shown: number;
  coverage_since: number | null;
  coverage_until: number | null;
  channels: string[];
  model: string;
  schema: string;
  truncated: boolean;
};

export type RunOutcome =
  | { status: "cached" }
  | { status: "stalled"; reason: string; pending: number }
  | { status: "refused"; reason: string; model_output: string }
  | { status: "unpublished"; reason: string; model_output: string }
  | {
      status: "folded";
      artifact: Artifact;
      shown: number;
      pending: number;
      truncated: boolean;
    };

/** The daemon omits empty selection arrays on the wire (serde
 * `skip_serializing_if`); restore them so `Selection` is total in the app. */
function normalizeSelection(s: Partial<Selection> | undefined): Selection {
  return {
    channels: s?.channels ?? [],
    authors: s?.authors ?? [],
    kinds: s?.kinds ?? [],
  };
}

const BASE = "/api";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, init);
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // non-JSON error body; keep the status line
    }
    throw new Error(message);
  }
  return (await res.json()) as T;
}

function json(body: unknown): Pick<RequestInit, "headers" | "body"> {
  return {
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  };
}

function post(body: unknown): RequestInit {
  return { method: "POST", ...json(body) };
}

export const api = {
  status: () => request<Status>("/status"),
  channels: () => request<{ channels: Channel[] }>("/channels"),
  event: (id: string) => request<EventItem>(`/events/${id}`),
  preview: async (selection: Selection, window: TimeWindow) => {
    const p = await request<Preview>(
      "/select/preview",
      post({ selection, ...window }),
    );
    p.selection = normalizeSelection(p.selection);
    return p;
  },
  events: (
    selection: Selection,
    window: TimeWindow,
    limit: number,
    after: PageCursor | null,
  ) =>
    request<EventsPage>(
      "/select/events",
      post({ selection, ...window, limit, after }),
    ),
  folds: async () => {
    const r = await request<{ folds: FoldRow[] }>("/folds");
    for (const f of r.folds) {
      f.spec.selection = normalizeSelection(f.spec.selection);
    }
    return r;
  },
  putFold: (
    name: string,
    body: {
      selection: Selection;
      model: string;
      instructions?: string;
      meta?: unknown;
    },
  ) => request<{ saved: FoldSpec }>(`/folds/${name}`, { method: "PUT", ...json(body) }),
  deleteFold: (name: string) =>
    request<{ deleted: string }>(`/folds/${name}`, { method: "DELETE" }),
  preflight: (name: string, window: TimeWindow) =>
    request<Preflight>(`/folds/${name}/preflight`, post(window)),
  run: (name: string, window: TimeWindow) =>
    request<RunOutcome>(`/folds/${name}/run`, post(window)),
  artifacts: (name: string) =>
    request<{ fold: string; artifacts: ArtifactSummary[] }>(
      `/folds/${name}/artifacts`,
    ),
  artifact: async (name: string, version: number) => {
    const a = await request<Artifact>(`/folds/${name}/artifacts/${version}`);
    a.selection = normalizeSelection(a.selection);
    return a;
  },
};

export function selectionIsRunnable(s: Selection): boolean {
  return s.channels.length > 0 || s.authors.length > 0;
}

export function shortId(id: string): string {
  return id.slice(0, 8);
}

export function fmtTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function fmtDay(ts: number): string {
  return new Date(ts * 1000).toLocaleDateString(undefined, {
    timeZone: "UTC",
    month: "short",
    day: "numeric",
  });
}

export function fmtTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`;
}
