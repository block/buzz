import type { HostAgentStatus, RemoteAgentPreset } from "./types";

export class HostAgentdError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "HostAgentdError";
    this.status = status;
  }
}

function authHeaders(token: string): HeadersInit {
  return {
    Authorization: `Bearer ${token}`,
    Accept: "application/json",
  };
}

function normalizeBaseUrl(baseUrl: string): string {
  return baseUrl.replace(/\/$/, "");
}

/** Map low-level fetch failures into an actionable Remote Agents message. */
export function formatHostAgentdNetworkError(
  err: unknown,
  baseUrl: string,
): string {
  const raw =
    err instanceof Error
      ? err.message
      : typeof err === "string"
        ? err
        : "request failed";
  const lower = raw.toLowerCase();
  // Chromium / WebKit: "Failed to fetch" / "Load failed" / "NetworkError"
  if (
    lower.includes("failed to fetch") ||
    lower.includes("load failed") ||
    lower.includes("networkerror") ||
    lower.includes("network request failed") ||
    lower.includes("fetch failed")
  ) {
    return (
      `Cannot reach host-agentd at ${normalizeBaseUrl(baseUrl)}. ` +
      "Prefer the home Tailscale IP (e.g. http://100.79.175.63:8787) with " +
      "HOST_AGENTD_HOST bound to that IP on home — not laptop 127.0.0.1 unless " +
      "you intentionally run an SSH local forward. Prove with: " +
      'curl -sS -H "Authorization: Bearer <token>" <baseUrl>/v1/health ' +
      '(expect {"ok":true}). Token is only checked after TCP connects.'
    );
  }
  if (lower.includes("cors") || lower.includes("access-control")) {
    return (
      `CORS blocked ${normalizeBaseUrl(baseUrl)}. host-agentd needs CORS headers ` +
      "(restart home daemon after updating host-agentd.py)."
    );
  }
  return raw;
}

async function hostFetch(
  baseUrl: string,
  path: string,
  init: RequestInit,
): Promise<Response> {
  const url = `${normalizeBaseUrl(baseUrl)}${path}`;
  try {
    return await fetch(url, init);
  } catch (err) {
    throw new HostAgentdError(formatHostAgentdNetworkError(err, baseUrl), 0);
  }
}

async function parseJson(res: Response): Promise<unknown> {
  const text = await res.text();
  try {
    return text ? JSON.parse(text) : {};
  } catch {
    return { raw: text };
  }
}

export async function hostAgentdHealth(
  baseUrl: string,
  token: string,
): Promise<{ ok: boolean; service?: string }> {
  const res = await hostFetch(baseUrl, "/v1/health", {
    headers: authHeaders(token),
  });
  const body = (await parseJson(res)) as { ok?: boolean; service?: string };
  if (!res.ok) {
    throw new HostAgentdError(
      (body as { error?: string }).error || `health ${res.status}`,
      res.status,
    );
  }
  return { ok: Boolean(body.ok), service: body.service };
}

export async function hostAgentdStatus(
  baseUrl: string,
  token: string,
): Promise<HostAgentStatus> {
  const res = await hostFetch(baseUrl, "/v1/status", {
    headers: authHeaders(token),
  });
  const body = (await parseJson(res)) as HostAgentStatus;
  if (!res.ok) {
    throw new HostAgentdError(body.error || `status ${res.status}`, res.status);
  }
  return body;
}

export type CreateRemoteAgentInput = {
  seatId?: string;
  displayName: string;
  model: string;
  preset: RemoteAgentPreset;
  room?: string;
  notes?: string;
  arm?: boolean;
};

export async function hostAgentdCreateAgent(
  baseUrl: string,
  token: string,
  input: CreateRemoteAgentInput,
): Promise<{
  ok: boolean;
  seat_id?: string;
  model?: string;
  armed?: boolean;
  error?: string;
}> {
  const res = await hostFetch(baseUrl, "/v1/agents", {
    method: "POST",
    headers: {
      ...authHeaders(token),
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      seat_id: input.seatId || undefined,
      display_name: input.displayName,
      model: input.model,
      preset: input.preset,
      room: input.room || undefined,
      notes: input.notes || undefined,
      arm: input.arm !== false,
    }),
  });
  const body = (await parseJson(res)) as {
    ok?: boolean;
    seat_id?: string;
    model?: string;
    armed?: boolean;
    error?: string;
    arm?: { ok?: boolean; stderr?: string };
  };
  if (!res.ok || body.ok === false) {
    throw new HostAgentdError(
      formatDualBodyError(body, res.status) ||
        body.error ||
        body.arm?.stderr ||
        `create ${res.status}`,
      res.status,
    );
  }
  return {
    ok: true,
    seat_id: body.seat_id,
    model: body.model,
    armed: body.armed,
  };
}

/** Human-readable dual_body (409) — never silent second spawn. */
function formatDualBodyError(
  body: {
    error?: string;
    message?: string;
    place_proof?: {
      birth_cert_id?: string;
      host_id?: string;
      host_role?: string;
      body_id?: string | null;
      surface_kind?: string;
      health?: string;
    };
  },
  status: number,
): string | null {
  if (body.error !== "dual_body" && status !== 409) return null;
  const pp = body.place_proof;
  const dna = pp?.birth_cert_id
    ? `${pp.birth_cert_id.slice(0, 8)}…`
    : "unknown";
  const place = [pp?.host_id, pp?.host_role, pp?.surface_kind]
    .filter(Boolean)
    .join(" · ");
  return (
    body.message ||
    `Refuse dual body (DNA ${dna}${place ? ` · live on ${place}` : ""}). ` +
      "Adopt the existing home body or fork a new birth certificate — " +
      "do not silently spawn a second instance."
  );
}

export async function hostAgentdArm(
  baseUrl: string,
  token: string,
  seatId: string,
  preset: RemoteAgentPreset,
  room?: string,
  model?: string,
): Promise<{ ok: boolean; stdout?: string; stderr?: string }> {
  const res = await hostFetch(
    baseUrl,
    `/v1/agents/${encodeURIComponent(seatId)}/arm`,
    {
      method: "POST",
      headers: {
        ...authHeaders(token),
        "Content-Type": "application/json",
      },
      body: JSON.stringify({
        preset,
        room: room || undefined,
        model: model || undefined,
      }),
    },
  );
  const body = (await parseJson(res)) as {
    ok?: boolean;
    error?: string;
    message?: string;
    stdout?: string;
    stderr?: string;
    place_proof?: {
      birth_cert_id?: string;
      host_id?: string;
      host_role?: string;
      body_id?: string | null;
      surface_kind?: string;
      health?: string;
    };
  };
  if (!res.ok || body.ok === false) {
    throw new HostAgentdError(
      formatDualBodyError(body, res.status) ||
        body.error ||
        body.stderr ||
        `arm ${res.status}`,
      res.status,
    );
  }
  return { ok: true, stdout: body.stdout, stderr: body.stderr };
}

export async function hostAgentdLocationProof(
  baseUrl: string,
  token: string,
  view: "full" | "public" = "full",
): Promise<Record<string, unknown>> {
  const q = view === "public" ? "?view=public" : "";
  const res = await hostFetch(baseUrl, `/v1/location-proof${q}`, {
    headers: authHeaders(token),
  });
  const body = (await parseJson(res)) as Record<string, unknown>;
  if (!res.ok) {
    throw new HostAgentdError(
      (body.error as string) || `location-proof ${res.status}`,
      res.status,
    );
  }
  return body;
}

export async function hostAgentdDisarm(
  baseUrl: string,
  token: string,
  seatId: string,
  preset: RemoteAgentPreset,
): Promise<{ ok: boolean; stdout?: string; stderr?: string }> {
  const res = await hostFetch(
    baseUrl,
    `/v1/agents/${encodeURIComponent(seatId)}/disarm`,
    {
      method: "POST",
      headers: {
        ...authHeaders(token),
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ preset }),
    },
  );
  const body = (await parseJson(res)) as {
    ok?: boolean;
    error?: string;
    stdout?: string;
    stderr?: string;
  };
  if (!res.ok || body.ok === false) {
    throw new HostAgentdError(
      body.error || body.stderr || `disarm ${res.status}`,
      res.status,
    );
  }
  return { ok: true, stdout: body.stdout, stderr: body.stderr };
}
