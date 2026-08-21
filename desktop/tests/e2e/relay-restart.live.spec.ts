import { execFile } from "node:child_process";
import { promisify } from "node:util";

import { expect, test, type Page, bootstrapE2ePage } from "../helpers/test";

import { installBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { TwoRelayHarness, type RelaySpec } from "./helpers/twoRelayHarness";

const exec = promisify(execFile);

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

async function runCli(args: string[], relayUrl: string, privateKey: string) {
  const binary = required("BUZZ_E2E_CLI_BIN", process.env.BUZZ_E2E_CLI_BIN);
  const { stdout } = await exec(binary, args, {
    cwd: "..",
    env: {
      ...process.env,
      BUZZ_AUTH_TAG: "",
      BUZZ_PRIVATE_KEY: privateKey,
      BUZZ_RELAY_URL: relayUrl,
    },
  });
  return stdout;
}

async function seedLiveChannel(relayUrl: string) {
  const name = `reconnect-live-${process.pid}`;
  const created = JSON.parse(
    await runCli(
      [
        "channels",
        "create",
        "--name",
        name,
        "--type",
        "stream",
        "--visibility",
        "open",
      ],
      relayUrl,
      TEST_IDENTITIES.alice.privateKey,
    ),
  ) as { channel_id: string };
  await runCli(
    [
      "channels",
      "add-member",
      "--channel",
      created.channel_id,
      "--pubkey",
      TEST_IDENTITIES.tyler.pubkey,
      "--role",
      "member",
    ],
    relayUrl,
    TEST_IDENTITIES.alice.privateKey,
  );
  return { id: created.channel_id, name };
}

async function connectionState(page: Page): Promise<string> {
  return page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
    };
    return win.__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.() ?? "uninstalled";
  });
}

async function exerciseBackgroundTraffic(page: Page, durationMs: number) {
  await page.evaluate(async (duration) => {
    const deadline = Date.now() + duration;
    while (Date.now() < deadline) {
      void window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
      await new Promise((resolve) => window.setTimeout(resolve, 100));
    }
  }, durationMs);
}

async function resetConnectAttempts(page: Page) {
  await page.evaluate(() => {
    window.__BUZZ_E2E_RESET_WEBSOCKET_CONNECT_ATTEMPTS__?.();
  });
}

async function assertConnectAttemptsArePaced(page: Page) {
  const attempts = await page.evaluate(
    () => window.__BUZZ_E2E_GET_WEBSOCKET_CONNECT_ATTEMPTS__?.() ?? [],
  );
  expect(attempts.length).toBeGreaterThanOrEqual(2);
  expect(attempts.length).toBeLessThanOrEqual(4);
  for (let index = 1; index < attempts.length; index += 1) {
    expect(attempts[index] - attempts[index - 1]).toBeGreaterThanOrEqual(700);
  }
}

async function proveLiveDelivery(
  page: Page,
  relayUrl: string,
  channel: { id: string; name: string },
  label: string,
) {
  await page.getByTestId(`channel-${channel.name}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channel.name);
  const message = `${label} ${Date.now()}`;
  await runCli(
    ["messages", "send", "--channel", channel.id, "--content", message],
    relayUrl,
    TEST_IDENTITIES.alice.privateKey,
  );
  await expect(page.getByTestId("message-timeline")).toContainText(message, {
    timeout: 30_000,
  });
}

test.describe("relay restart live gate", () => {
  test.skip(!enabled, "set BUZZ_E2E_RELAY_RESTART=1 to run live gate");

  test("client reconnects after the relay is SIGTERMed and restarted", async ({
    page,
  }) => {
    test.setTimeout(240_000);
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
      const channel = await test.step("seed live channel and membership", () =>
        seedLiveChannel(relayHttpUrl));
      await installBridge(page, {
        mode: "relay",
        user: "tyler",
        relayHttpUrl,
        relayWsUrl: `ws://127.0.0.1:${spec.ports.main}`,
      });
      await bootstrapE2ePage(page, "/");

      // Baseline: the app converges to a live authenticated session and sees
      // the channel created for this fresh database.
      await test.step("wait for initial authenticated connection", async () => {
        await expect
          .poll(() => connectionState(page), { timeout: 60_000 })
          .toBe("connected");
        await expect(page.getByTestId(`channel-${channel.name}`)).toBeVisible({
          timeout: 30_000,
        });
      });

      // Roll the pod. Graceful drain: readiness 503 → 5s grace → 1012 close
      // broadcast → process exit. The client must observe the close (not a
      // silent stall) and start retrying.
      await resetConnectAttempts(page);
      await harness.terminateRelayGracefully(spec.name);
      await expect
        .poll(() => connectionState(page), { timeout: 30_000 })
        .not.toBe("connected");

      // Keep the real relay unavailable across several reconnect windows while
      // ordinary app traffic continues. This is the production-shaped race:
      // background queries must not bypass the session coordinator's backoff.
      await exerciseBackgroundTraffic(page, 8_000);
      await expect.poll(() => connectionState(page)).not.toBe("connected");
      await assertConnectAttemptsArePaced(page);

      // Bring the "new pod" up on the same address, exactly like a k8s
      // restart behind a stable service endpoint.
      await harness.restartRelay(spec.name);

      // The client's retry loop must find the fresh relay and converge back
      // to connected without any user interaction, then prove that AUTH and
      // live-subscription replay finished by receiving an event published by
      // a second identity through the real CLI/relay boundary.
      await expect
        .poll(() => connectionState(page), { timeout: 60_000 })
        .toBe("connected");
      await proveLiveDelivery(
        page,
        relayHttpUrl,
        channel,
        "first automatic recovery",
      );

      // Flap the fresh pod once more. A second recovery catches stale timer,
      // waiter, generation, and subscription state that a single cycle cannot.
      await harness.terminateRelayGracefully(spec.name);
      await expect
        .poll(() => connectionState(page), { timeout: 30_000 })
        .not.toBe("connected");
      await exerciseBackgroundTraffic(page, 4_000);
      await harness.restartRelay(spec.name);
      await expect
        .poll(() => connectionState(page), { timeout: 60_000 })
        .toBe("connected");
      await proveLiveDelivery(
        page,
        relayHttpUrl,
        channel,
        "second automatic recovery",
      );
    } catch (error) {
      console.error(await harness.logs());
      throw error;
    } finally {
      await harness.stop();
    }
  });
});
