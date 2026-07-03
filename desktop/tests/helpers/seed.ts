import { request } from "@playwright/test";

const tylerPubkey =
  "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34";
const isCi = Boolean(process.env.CI);
// Multi-tenant: the relay resolves each request's tenant from its Host header
// against the communities host map and fails closed (404) on an unmapped host.
// The seed maps host 'localhost:3000' (matching the relay's RELAY_URL) — and
// `localhost` != `127.0.0.1` to normalize_host — so this readiness check must
// hit `localhost:3000`, not `127.0.0.1:3000`, or every /query 404s. This also
// matches the rest of the desktop e2e suite (e2eBridge.ts / bridge.ts), which
// already default to localhost.
const relayBaseUrl = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";
const seedTimeoutMs = Number.parseInt(
  process.env.BUZZ_E2E_SEED_TIMEOUT_MS ?? (isCi ? "60000" : "25000"),
  10,
);
const requestTimeoutMs = Number.parseInt(
  process.env.BUZZ_E2E_SEED_REQUEST_TIMEOUT_MS ?? (isCi ? "5000" : "2000"),
  10,
);
const retryDelayMs = Number.parseInt(
  process.env.BUZZ_E2E_SEED_RETRY_DELAY_MS ?? "1000",
  10,
);

function delay(ms: number) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

export async function assertRelaySeeded() {
  const context = await request.newContext();
  const deadline = Date.now() + seedTimeoutMs;
  let lastFailure = "no checks attempted";

  try {
    while (Date.now() < deadline) {
      try {
        // The setup script inserts test data directly into the DB tables.
        // The relay reconciles these at startup by emitting kind:39000/39002
        // events. Query kind:39000 (channel metadata) via the HTTP bridge
        // and check for the expected "general" channel.
        const response = await context.post(`${relayBaseUrl}/query`, {
          headers: {
            "X-Pubkey": tylerPubkey,
            "Content-Type": "application/json",
          },
          data: [{ kinds: [39000], limit: 200 }],
          timeout: requestTimeoutMs,
        });

        if (!response.ok()) {
          lastFailure = `HTTP ${response.status()} from POST /query`;
        } else {
          const events = (await response.json()) as Array<{
            tags: string[][];
          }>;
          const hasGeneral = events.some((event) =>
            event.tags.some((tag) => tag[0] === "name" && tag[1] === "general"),
          );
          if (hasGeneral) {
            return;
          }
          lastFailure = `seed data: got ${events.length} channels but no "general" — relay may still be reconciling`;
        }
      } catch (error) {
        lastFailure =
          error instanceof Error ? error.message : "unknown relay check error";
      }

      await delay(retryDelayMs);
    }

    throw new Error(
      `Relay test data was not ready after ${seedTimeoutMs}ms. Last check: ${lastFailure}. Start the relay and run scripts/setup-desktop-test-data.sh.`,
    );
  } finally {
    await context.dispose();
  }
}
