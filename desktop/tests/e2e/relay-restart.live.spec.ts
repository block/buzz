import { expect, test, type Page } from "@playwright/test";

import { installBridge } from "../helpers/bridge";
import { TwoRelayHarness, type RelaySpec } from "./helpers/twoRelayHarness";

// Live gate: boots a REAL buzz-relay process, points the app at it, SIGTERMs
// the relay mid-session, restarts it on the same port, and asserts the client
// converges back to "connected". This proves the full restart story end to
// end: the relay's graceful-drain 1012 close broadcast (server side) and the
// client's dial-failure retry + 1012 fast-reconnect (desktop side) — the two
// halves that synthetic mock-websocket specs cannot compose.
//
// Requires: BUZZ_E2E_RELAY_RESTART=1, BUZZ_E2E_RELAY_BIN, and
// BUZZ_E2E_DATABASE_URL (plus reachable Redis and media object store, same
// infra as the agents-everywhere live gate).
const enabled = process.env.BUZZ_E2E_RELAY_RESTART === "1";

function required(name: string, value: string | undefined): string {
  if (!value) throw new Error(`${name} is required for the live gate`);
  return value;
}

async function connectionState(page: Page): Promise<string> {
  return page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
    };
    return win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.() ?? "uninstalled";
  });
}

test.describe("relay restart live gate", () => {
  test.skip(!enabled, "set BUZZ_E2E_RELAY_RESTART=1 to run live gate");

  test("client reconnects after the relay is SIGTERMed and restarted", async ({
    page,
  }) => {
    test.setTimeout(180_000);
    const portBase = 26_000 + (process.pid % 3_000);
    const spec: RelaySpec = {
      name: "relay-restart",
      ports: {
        main: portBase,
        health: portBase + 3_000,
        metrics: portBase + 6_000,
      },
      databaseUrl: required(
        "BUZZ_E2E_DATABASE_URL",
        process.env.BUZZ_E2E_DATABASE_URL,
      ),
      redisUrl:
        process.env.BUZZ_E2E_REDIS_RESTART ?? "redis://127.0.0.1:6379/13",
    };
    const harness = await TwoRelayHarness.create([spec]);
    try {
      await harness.startRelays();

      const relayHttpUrl = `http://127.0.0.1:${spec.ports.main}`;
      await installBridge(page, {
        mode: "relay",
        user: "tyler",
        relayHttpUrl,
        relayWsUrl: `ws://127.0.0.1:${spec.ports.main}`,
      });
      await page.goto("/");

      // Baseline: the app converges to a live authenticated session.
      await expect
        .poll(() => connectionState(page), { timeout: 60_000 })
        .toBe("connected");

      // Roll the pod. Graceful drain: readiness 503 → 5s grace → 1012 close
      // broadcast → process exit. The client must observe the close (not a
      // silent stall) and start retrying.
      await harness.terminateRelayGracefully(spec.name);
      await expect
        .poll(() => connectionState(page), { timeout: 30_000 })
        .not.toBe("connected");

      // Bring the "new pod" up on the same address, exactly like a k8s
      // restart behind a stable service endpoint.
      await harness.restartRelay(spec.name);

      // The client's retry loop must find the fresh relay and converge back
      // to connected without any user interaction.
      await expect
        .poll(() => connectionState(page), { timeout: 60_000 })
        .toBe("connected");
    } catch (error) {
      console.error(await harness.logs());
      throw error;
    } finally {
      await harness.stop();
    }
  });
});
