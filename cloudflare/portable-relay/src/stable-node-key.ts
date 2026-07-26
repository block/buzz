const SUPPORTED_SCHEMES = new Set(["http:", "https:", "ws:", "wss:"]);
const MAX_STABLE_NODE_KEY_LENGTH = 255;

export class StableNodeKeyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StableNodeKeyError";
  }
}

/**
 * Derives the durable relay boundary from a URL authority.
 *
 * URL parsing removes only the default port for the URL's own scheme. The
 * remaining normalization makes hostname case and a terminal DNS dot
 * insignificant while preserving non-default ports.
 */
export function stableNodeKeyFromUrl(input: string | URL): string {
  let url: URL;
  try {
    url = typeof input === "string" ? new URL(input) : input;
  } catch {
    throw new StableNodeKeyError("stable node authority is not a valid URL");
  }

  if (!SUPPORTED_SCHEMES.has(url.protocol.toLowerCase())) {
    throw new StableNodeKeyError(
      `unsupported stable node scheme: ${url.protocol}`,
    );
  }
  if (url.username !== "" || url.password !== "") {
    throw new StableNodeKeyError(
      "stable node authority must not contain credentials",
    );
  }

  const hostname = normalizeHostname(url.hostname);
  if (hostname === "") {
    throw new StableNodeKeyError("stable node authority has no hostname");
  }

  const key = url.port === "" ? hostname : `${hostname}:${url.port}`;
  if (key.length > MAX_STABLE_NODE_KEY_LENGTH) {
    throw new StableNodeKeyError("stable node key exceeds 255 characters");
  }
  return key;
}

function normalizeHostname(hostname: string): string {
  const lower = hostname.toLowerCase();
  if (lower.startsWith("[") && lower.endsWith("]")) {
    return lower;
  }
  return lower.replace(/\.+$/, "");
}
