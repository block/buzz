export type BusinessHostAuthMessage = {
  version: 3;
  type: "HOST_INIT" | "CHECK_AUTH" | "LOGOUT";
  requestId: string;
  sessionNonce: string;
  payload?: unknown;
};

type BusinessSession = {
  authenticated: true;
  subject: string;
  displayName: string;
};

const MAX_REQUEST_ID = 128;
const MAX_SESSION_NONCE = 256;

function boundedText(value: unknown, max: number): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= max;
}

export function parseBusinessHostAuthMessage(
  value: unknown,
): BusinessHostAuthMessage | null {
  if (typeof value !== "object" || value === null || Array.isArray(value))
    return null;
  const message = value as Record<string, unknown>;
  if (
    message.version !== 3 ||
    !boundedText(message.requestId, MAX_REQUEST_ID) ||
    !boundedText(message.sessionNonce, MAX_SESSION_NONCE) ||
    (message.type !== "HOST_INIT" &&
      message.type !== "CHECK_AUTH" &&
      message.type !== "LOGOUT")
  )
    return null;
  return {
    version: 3,
    type: message.type,
    requestId: message.requestId,
    sessionNonce: message.sessionNonce,
    payload: message.payload,
  };
}

function parentOrigin(): string | null {
  if (window.parent === window) return null;
  try {
    const referrer = new URL(document.referrer);
    if (!isAllowedBusinessHostProtocol(referrer.protocol)) return null;
    return referrer.protocol === "tauri:"
      ? `${referrer.protocol}//${referrer.host}`
      : referrer.origin;
  } catch {
    return null;
  }
}

export function isAllowedBusinessHostProtocol(protocol: string): boolean {
  return protocol === "https:" || protocol === "http:" || protocol === "tauri:";
}

function isAllowedBusinessHostOrigin(origin: string): boolean {
  try {
    return isAllowedBusinessHostProtocol(new URL(origin).protocol);
  } catch {
    return false;
  }
}

async function readBusinessSession(): Promise<BusinessSession | null> {
  const response = await fetch("/api/session", {
    credentials: "include",
    headers: { accept: "application/json" },
  });
  if (response.status === 401) return null;
  if (!response.ok) throw new Error(`Business session check failed (${response.status})`);
  const body = (await response.json()) as Record<string, unknown>;
  if (
    body.authenticated !== true ||
    !boundedText(body.subject, 256) ||
    !boundedText(body.displayName, 180)
  )
    throw new Error("Business session response was invalid");
  return {
    authenticated: true,
    subject: body.subject,
    displayName: body.displayName,
  };
}

export async function logoutBusinessSession(
  fetchImpl: typeof fetch = fetch,
): Promise<void> {
  const sessionResponse = await fetchImpl("/api/session", {
    credentials: "include",
    headers: { accept: "application/json" },
  });
  if (sessionResponse.status === 401) return;
  if (!sessionResponse.ok)
    throw new Error(`Business session check failed (${sessionResponse.status})`);
  const session = (await sessionResponse.json()) as Record<string, unknown>;
  if (!boundedText(session.csrfToken, 512))
    throw new Error("Business session response did not include CSRF");
  const logoutResponse = await fetchImpl("/api/logout", {
    method: "POST",
    credentials: "include",
    headers: { "x-csrf-token": session.csrfToken },
  });
  if (!logoutResponse.ok)
    throw new Error(`Business logout failed (${logoutResponse.status})`);
}

export function connectBusinessDockAuthBridge(): () => void {
  if (window.parent === window) return () => undefined;
  const knownOrigin = parentOrigin();
  let authenticated = false;

  const post = (
    request: BusinessHostAuthMessage,
    type: "AUTH_STATUS" | "AUTH_REQUIRED" | "SESSION_EXPIRED",
    payload: unknown,
    targetOrigin: string,
  ) => {
    window.parent.postMessage(
      {
        version: 3,
        type,
        requestId: request.requestId,
        sessionNonce: request.sessionNonce,
        payload,
      },
      targetOrigin,
    );
  };

  const check = async (
    request: BusinessHostAuthMessage,
    targetOrigin: string,
  ) => {
    try {
      const session = await readBusinessSession();
      if (!session) {
        post(
          request,
          authenticated ? "SESSION_EXPIRED" : "AUTH_REQUIRED",
          { reason: "Business session is required" },
          targetOrigin,
        );
        authenticated = false;
        return;
      }
      authenticated = true;
      post(request, "AUTH_STATUS", {
        authenticated: true,
        user: { subject: session.subject, displayName: session.displayName },
      }, targetOrigin);
    } catch {
      post(request, "AUTH_REQUIRED", {
        reason: "Business session could not be verified",
      }, targetOrigin);
    }
  };

  const onMessage = (event: MessageEvent) => {
    if (event.source !== window.parent) return;
    if (
      knownOrigin !== null
        ? event.origin !== knownOrigin
        : event.origin !== "null" &&
          !isAllowedBusinessHostOrigin(event.origin)
    )
      return;
    const request = parseBusinessHostAuthMessage(event.data);
    if (!request) return;
    if (request.type === "HOST_INIT" || request.type === "CHECK_AUTH")
      void check(request, event.origin === "null" ? "*" : event.origin);
    else if (request.type === "LOGOUT")
      void logoutBusinessSession()
        .then(() => {
          authenticated = false;
          post(
            request,
            "AUTH_STATUS",
            { authenticated: false },
            event.origin === "null" ? "*" : event.origin,
          );
        })
        .catch(() => {
          post(
            request,
            "AUTH_REQUIRED",
            { reason: "Business session could not be logged out" },
            event.origin === "null" ? "*" : event.origin,
          );
        });
  };
  window.addEventListener("message", onMessage);
  return () => window.removeEventListener("message", onMessage);
}
