export function parseNativeReviewRelay(relayUrl: string): URL | null {
  let relay: URL;
  try {
    relay = new URL(relayUrl);
  } catch {
    return null;
  }
  const port = Number(relay.port);
  const loopbackHost = ["localhost", "127.0.0.1", "[::1]"].includes(
    relay.hostname,
  );
  if (
    !["ws:", "http:"].includes(relay.protocol) ||
    !loopbackHost ||
    !Number.isInteger(port) ||
    port < 1 ||
    port > 65_535 ||
    (relay.pathname !== "/" && relay.pathname !== "") ||
    relay.username ||
    relay.password ||
    relay.search ||
    relay.hash
  ) {
    return null;
  }
  return relay;
}

export function isNativeReviewProbeConfig(
  probeUrl: string,
  probeToken: string,
): boolean {
  let probe: URL;
  try {
    probe = new URL(probeUrl);
  } catch {
    return false;
  }
  const port = Number(probe.port);
  return (
    probe.protocol === "http:" &&
    probe.hostname === "127.0.0.1" &&
    Number.isInteger(port) &&
    port >= 1 &&
    port <= 65_535 &&
    probe.pathname === "/snapshot" &&
    !probe.username &&
    !probe.password &&
    !probe.search &&
    !probe.hash &&
    Boolean(probeToken)
  );
}
