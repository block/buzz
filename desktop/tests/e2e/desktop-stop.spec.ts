import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

// These are IPC fixtures, not native execution evidence. The real mounted
// Known Desktops, client, relay publisher and retry control remain in the path.
test("remote Stop distinguishes delivery, uncertainty, and confirmed result", async ({
  page,
}) => {
  test.setTimeout(60_000);
  const agent = "a7".repeat(32);
  await installMockBridge(page, {
    managedAgents: [],
    relayAgents: [
      {
        pubkey: agent,
        name: "Scout",
        ownerPubkey: "deadbeef".repeat(8),
        status: "unknown",
        respondTo: "owner-only",
        channelNames: [],
        channelIds: [],
      },
    ],
  });
  await page.goto("/");
  await expect(page.getByTestId("open-agents-view")).toBeVisible();
  await page.evaluate(() => {
    const w = window as typeof window & {
      __STOP_FIXTURE__: {
        confirmed: boolean;
        prepared: number;
        sends: string[];
        lifecyclePrepared: number;
        lifecycleSends: string[];
      };
      __TAURI_INTERNALS__: {
        invoke: (command: string, payload?: any, options?: any) => Promise<any>;
      };
    };
    w.__STOP_FIXTURE__ = {
      confirmed: false,
      prepared: 0,
      sends: [],
      lifecyclePrepared: 0,
      lifecycleSends: [],
    };
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    const now = Math.floor(Date.now() / 1000);
    const local = "11111111-1111-4111-8111-111111111111";
    const remote = "22222222-2222-4222-8222-222222222222";
    const sign = async (kind: number, tags: string[][] = []) =>
      JSON.parse(
        await original("sign_event", {
          kind,
          tags,
          content: "encrypted IPC fixture",
          createdAt: now,
        }),
      );
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      switch (command) {
        case "prepare_desktop_profile":
          return { event: await sign(30180, [["d", local]]) };
        case "read_desktop_profiles":
          return [
            { id: local, name: "Laptop", updated: now },
            { id: remote, name: "Lab Desktop", updated: now },
          ];
        case "prepare_desktop_observation":
          return { event: await sign(30181, [["d", local]]) };
        case "read_desktop_observations":
          return [
            { id: local, heard: now },
            { id: remote, heard: now - 600 },
          ];
        case "prepare_desktop_capabilities":
          return { event: await sign(30182, [["d", local]]) };
        case "read_desktop_capabilities":
          return [local, remote].map((id) => ({
            id,
            reported: now,
            runtimes: [],
          }));
        case "observe_desktop_placement":
          return;
        case "read_desktop_placement":
        case "receive_desktop_lifecycle":
          return null;
        case "prepare_desktop_lifecycle":
          w.__STOP_FIXTURE__.lifecyclePrepared++;
          return sign(50182, [
            ["p", payload.owner],
            ["d", payload.desktop],
          ]);
        case "read_desktop_lifecycle_results":
          return "provisioning_unavailable";
        case "prepare_desktop_stop":
          w.__STOP_FIXTURE__.prepared++;
          return sign(50180, [
            ["p", payload.owner],
            ["d", payload.desktop],
          ]);
        case "receive_desktop_stop":
          return null;
        case "read_desktop_stop_results":
          return w.__STOP_FIXTURE__.confirmed ? "stopped" : "unknown";
        case "plugin:websocket|send": {
          const wire = JSON.parse(payload.message.data);
          if (wire[0] === "EVENT" && wire[1]?.kind === 50180)
            w.__STOP_FIXTURE__.sends.push(JSON.stringify(wire[1]));
          if (wire[0] === "EVENT" && wire[1]?.kind === 50182)
            w.__STOP_FIXTURE__.lifecycleSends.push(JSON.stringify(wire[1]));
          break;
        }
      }
      return original(command, payload, options);
    };
  });
  await page.getByTestId("open-agents-view").click();
  const desktops = page.getByRole("region", { name: "Known Desktops" });
  await desktops.getByRole("button", { name: "Refresh", exact: true }).click();
  await expect(
    desktops.getByRole("listitem").getByText("Lab Desktop", { exact: true }),
  ).toBeVisible();
  await desktops
    .getByRole("combobox", { name: "Agent to stop on Lab Desktop" })
    .selectOption(agent);
  await waitForAnimations(page);
  await desktops.screenshot({
    path: "test-results/desktop-stop/01-selected.png",
  });

  await desktops
    .getByRole("button", { name: "Stop on Lab Desktop", exact: true })
    .click();
  await expect(
    desktops.getByText("Stop requested. Waiting for this Desktop’s result…", {
      exact: true,
    }),
  ).toBeVisible();
  await expect(
    desktops.getByText("Stop confirmed by Lab Desktop.", { exact: true }),
  ).toHaveCount(0);
  await waitForAnimations(page);
  await desktops.screenshot({
    path: "test-results/desktop-stop/02-waiting.png",
  });

  await expect(
    desktops.getByText(
      "Stop unconfirmed. This Desktop may be unavailable; its agents may still be running.",
      { exact: true },
    ),
  ).toBeVisible({ timeout: 25_000 });
  await waitForAnimations(page);
  await desktops.screenshot({
    path: "test-results/desktop-stop/03-unconfirmed.png",
  });
  await page.evaluate(() => {
    (
      window as typeof window & { __STOP_FIXTURE__: { confirmed: boolean } }
    ).__STOP_FIXTURE__.confirmed = true;
  });
  await desktops
    .getByRole("button", { name: "Retry same Stop", exact: true })
    .click();
  await expect(
    desktops.getByText("Stop confirmed by Lab Desktop.", { exact: true }),
  ).toBeVisible();
  const result = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __STOP_FIXTURE__: { prepared: number; sends: string[] };
        }
      ).__STOP_FIXTURE__,
  );
  expect(result.prepared).toBe(1);
  expect(result.sends).toHaveLength(2);
  expect(result.sends[1]).toBe(result.sends[0]);
  await waitForAnimations(page);
  await desktops.screenshot({
    path: "test-results/desktop-stop/04-confirmed.png",
  });

  // The mounted lifecycle selector shares host labels with the Stop rows.
  // IPC explicitly refuses launch; no native process is created by this fixture.
  const controls = desktops.getByRole("region", {
    name: "Agent placement controls",
  });
  await controls
    .getByRole("combobox", { name: "Agent to place" })
    .selectOption(agent);
  await controls
    .getByRole("combobox", { name: "Destination Desktop" })
    .selectOption("22222222-2222-4222-8222-222222222222");
  await controls
    .getByRole("button", { name: "Start on destination", exact: true })
    .click();
  await expect(controls.getByRole("status")).toHaveText(
    "Destination keyless launch provisioning is unavailable. No new process was started.",
  );
  await waitForAnimations(page);
  await desktops.screenshot({
    path: "test-results/desktop-stop/05-launch-unavailable.png",
  });
  await controls
    .getByRole("button", { name: "Retry same request", exact: true })
    .click();
  await expect(controls.getByRole("status")).toHaveText(
    "Destination keyless launch provisioning is unavailable. No new process was started.",
  );
  const lifecycle = await page.evaluate(
    () =>
      (
        window as typeof window & {
          __STOP_FIXTURE__: {
            lifecyclePrepared: number;
            lifecycleSends: string[];
          };
        }
      ).__STOP_FIXTURE__,
  );
  expect(lifecycle.lifecyclePrepared).toBe(1);
  expect(lifecycle.lifecycleSends).toHaveLength(2);
  expect(lifecycle.lifecycleSends[1]).toBe(lifecycle.lifecycleSends[0]);
});
