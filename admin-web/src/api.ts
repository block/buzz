import { clearToken, getToken } from "./token";

const PREFIX = "/api/admin/v1";

export class ApiFailure extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}

/// The authentication mode the relay requires, discovered via the probe.
/// - `token`    — operator bearer token (session-stored, prompts on first visit)
/// - `nip98`    — NIP-98 HTTP Auth; each request signed with a NIP-07 extension
/// - `disabled` — relay returned 200 with no auth; no credential needed
export type AuthMode = "token" | "nip98" | "disabled";

/// Sign a NIP-98 kind-27235 event for the given URL + method via window.nostr.
/// Throws if window.nostr is not available or signing fails.
async function signNip98(url: string, method: string): Promise<string> {
  const nostr = (window as Window & typeof globalThis & { nostr?: Nostr98 })
    .nostr;
  if (!nostr) throw new Error("No NIP-07 extension available");
  const event = await nostr.signEvent({
    kind: 27235,
    created_at: Math.floor(Date.now() / 1000),
    tags: [
      ["u", url],
      ["method", method],
    ],
    content: "",
  });
  return `Nostr ${btoa(JSON.stringify(event))}`;
}

// Minimal type for the NIP-07 window.nostr interface.
interface Nostr98 {
  signEvent(event: {
    kind: number;
    created_at: number;
    tags: string[][];
    content: string;
  }): Promise<Record<string, unknown>>;
}

/// Every admin API call goes through here. Attaches the correct credential
/// for the current auth mode:
/// - `token` mode: Bearer header from session storage
/// - `nip98` mode: sign a kind-27235 event via NIP-07 for each request
/// - `disabled` mode: no credential
///
/// A 401 in token mode clears the stored token so the prompt re-appears.
/// A 401 in nip98 mode re-signs once (handles key rotation / clock skew)
/// and retries the request exactly once; a second 401 surfaces the error —
/// no infinite loop.
async function send(
  path: string,
  accept: string,
  authMode: AuthMode,
): Promise<Response> {
  const doRequest = async (extraHeaders?: Record<string, string>) => {
    const headers: Record<string, string> = { accept, ...extraHeaders };
    if (authMode === "token") {
      const token = getToken();
      if (token) headers.authorization = `Bearer ${token}`;
    } else if (authMode === "nip98") {
      const url = `${location.protocol}//${location.host}${PREFIX}${path}`;
      headers.authorization = await signNip98(url, "GET");
    }
    return fetch(`${PREFIX}${path}`, { credentials: "same-origin", headers });
  };

  let response = await doRequest();

  if (response.status === 401) {
    if (authMode === "token") {
      clearToken();
    } else if (authMode === "nip98") {
      // Re-sign with a fresh event and retry exactly once (handles clock
      // skew or key rotation). A second 401 surfaces the error below.
      response = await doRequest();
    }
  }

  if (response.status === 401) {
    throw new ApiFailure(401, "The admin credential was rejected.");
  }
  if (!response.ok) {
    const envelope = await response.json().catch(() => null);
    throw new ApiFailure(
      response.status,
      envelope?.error?.message ?? `Request failed (${response.status})`,
    );
  }
  return response;
}

export async function request<T>(path: string, authMode: AuthMode): Promise<T> {
  const response = await send(path, "application/json", authMode);
  return response.json() as Promise<T>;
}

/// Probe whether the relay requires authentication and what kind.
/// Issues one unauthenticated request and reads the WWW-Authenticate header:
/// - 200                       → `disabled` (no auth needed)
/// - 401 + WWW-Authenticate: Nostr → `nip98`
/// - 401 + WWW-Authenticate: Bearer (or any other 4xx/5xx) → `token`
export async function probeAuthMode(): Promise<AuthMode> {
  try {
    const response = await fetch(`${PREFIX}/reports`, {
      credentials: "same-origin",
      headers: { accept: "application/json" },
    });
    if (response.status === 200) return "disabled";
    if (response.status === 401) {
      const challenge = response.headers.get("www-authenticate") ?? "";
      if (challenge.trim().toLowerCase() === "nostr") return "nip98";
    }
    return "token";
  } catch {
    // Network error — default to token so the prompt is shown.
    return "token";
  }
}

/// Attachments cannot be fetched by `<img src>` or `<a href>` because those
/// carry no Authorization header. Callers render the object URL and must
/// revoke it when it is replaced or unmounted.
export async function requestObjectUrl(
  path: string,
  authMode: AuthMode,
): Promise<string> {
  const response = await send(path, "*/*", authMode);
  return URL.createObjectURL(await response.blob());
}
