const NON_TLS_WARNING =
  "This address uses unencrypted HTTP on your network. Model names, prompts, and tool results may be visible in transit.";

/** Warn for cleartext network endpoints while allowing ordinary loopback Ollama. */
export function ollamaEndpointSecurityWarning(endpoint: string): string | null {
  let url: URL;
  try {
    url = new URL(endpoint);
  } catch {
    return null;
  }
  if (url.protocol !== "http:") return null;
  const host = url.hostname.toLowerCase();
  if (
    host === "localhost" ||
    host === "127.0.0.1" ||
    host === "[::1]" ||
    host === "::1"
  ) {
    return null;
  }
  return NON_TLS_WARNING;
}
