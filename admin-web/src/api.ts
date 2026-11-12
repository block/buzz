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
/// - `nip98`    — NIP-98 HTTP Auth; each request signed with a NIP-07 extension
/// - `disabled` — relay returned 200 with no auth; no credential needed
export type AuthMode = "nip98" | "disabled";

/// Sign a NIP-98 kind-27235 event for the given URL + method via window.nostr.
/// Throws if window.nostr is not available or signing fails.
///
/// A fresh random `nonce` tag is generated on every call. Without it, two
/// same-URL requests in the same second produce byte-identical events (the
/// signed fields are only `u`, `method`, and `created_at` at 1-second
/// resolution), so their event IDs collide — the relay's NIP-98 replay guard
/// rejects the second, and the 401 retry re-signs the same fields and can
/// never recover. The verifier ignores unknown tags, so the nonce is inert to
/// verification and serves only to make each signed event unique.
///
/// For body-bearing methods the caller passes `body`; a `payload` tag carrying
/// the hex SHA-256 of the exact bytes is added and signed. The relay's verifier
/// rejects a body-bearing request whose `payload` tag is absent or mismatched
/// (auth.rs), so the hash must be over the identical bytes the request sends.
async function signNip98(
  url: string,
  method: string,
  body?: Uint8Array,
): Promise<string> {
  const nostr = (window as Window & typeof globalThis & { nostr?: Nostr98 })
    .nostr;
  if (!nostr) throw new Error("No NIP-07 extension available");
  const nonce = crypto.getRandomValues(new Uint8Array(16));
  const nonceHex = toHex(nonce);
  const tags: string[][] = [
    ["u", url],
    ["method", method],
    ["nonce", nonceHex],
  ];
  if (body !== undefined) {
    tags.push(["payload", await sha256Hex(body)]);
  }
  const event = await nostr.signEvent({
    kind: 27235,
    created_at: Math.floor(Date.now() / 1000),
    tags,
    content: "",
  });
  return `Nostr ${btoa(JSON.stringify(event))}`;
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return toHex(new Uint8Array(digest));
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
/// - `nip98` mode: sign a kind-27235 event via NIP-07 for each request
/// - `disabled` mode: no credential
///
/// A 401 in nip98 mode re-signs once (handles key rotation / clock skew)
/// and retries the request exactly once; a second 401 surfaces the error —
/// no infinite loop.
async function send(
  path: string,
  accept: string,
  authMode: AuthMode,
  init?: { method?: string; body?: Uint8Array; contentType?: string },
): Promise<Response> {
  const method = init?.method ?? "GET";
  const body = init?.body;
  const doRequest = async () => {
    const headers: Record<string, string> = { accept };
    if (init?.contentType) headers["content-type"] = init.contentType;
    if (authMode === "nip98") {
      const url = `${location.protocol}//${location.host}${PREFIX}${path}`;
      headers.authorization = await signNip98(url, method, body);
    }
    return fetch(`${PREFIX}${path}`, {
      method,
      credentials: "same-origin",
      headers,
      body: body as BodyInit | undefined,
    });
  };

  let response = await doRequest();

  if (response.status === 401 && authMode === "nip98") {
    // Re-sign with a fresh event and retry exactly once (handles clock
    // skew or key rotation). A second 401 surfaces the error below.
    response = await doRequest();
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

/// Send a body-bearing mutation (PATCH/PUT/POST) and parse the JSON response.
/// The body is serialized once and signed over those exact bytes so the NIP-98
/// `payload` tag matches what the relay verifies.
export async function mutate<T>(
  path: string,
  method: string,
  body: unknown,
  authMode: AuthMode,
): Promise<T> {
  const bytes = new TextEncoder().encode(JSON.stringify(body));
  const response = await send(path, "application/json", authMode, {
    method,
    body: bytes,
    contentType: "application/json",
  });
  return response.json() as Promise<T>;
}

/// Probe whether the relay requires authentication.
/// Issues one unauthenticated request:
/// - 200 → `disabled` (no auth needed)
/// - anything else → `nip98` (the only authenticated mode; fail-secure)
export async function probeAuthMode(): Promise<AuthMode> {
  try {
    const response = await fetch(`${PREFIX}/reports`, {
      credentials: "same-origin",
      headers: { accept: "application/json" },
    });
    return response.status === 200 ? "disabled" : "nip98";
  } catch {
    // Network error — default to nip98 so a credential is required.
    return "nip98";
  }
}

/// Attachments cannot be fetched by `<img src>` or `<a href>` because those
/// carry no Authorization header. Callers render the object URL and must
/// revoke it when it is replaced or unmounted.
///
/// Returns the server-verified `type` alongside the URL. The relay sniffs the
/// stored bytes and only labels verified passive raster images as `image/*`;
/// everything else is `application/octet-stream`. Callers must decide inline
/// rendering from this type, never from the untrusted reporter-supplied MIME.
export async function requestObjectUrl(
  path: string,
  authMode: AuthMode,
): Promise<{ url: string; type: string }> {
  const response = await send(path, "*/*", authMode);
  const blob = await response.blob();
  return { url: URL.createObjectURL(blob), type: blob.type };
}
