import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("mock socket REQs preserve exact readiness semantics", async ({
  page,
}) => {
  // Keep app-owned channel subscriptions out of the readiness result. The
  // bridge is installed, but locked boot never mounts the channel consumers.
  await installMockBridge(page, { identityLocked: true });
  await page.goto("/");
  await expect(page.getByTestId("keyring-locked")).toBeVisible();

  const observed = await page.evaluate(async () => {
    const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    const ready = window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__;
    if (!invoke || !ready) throw new Error("Mock socket bridge is unavailable");
    const channel = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
    const owner = "deadbeef".repeat(8);
    const frames: unknown[][] = [];
    const id = await invoke("plugin:websocket|connect", {
      onMessage: (batch: Array<{ type: string; data?: string }>) => {
        for (const frame of batch) {
          if (frame.type === "Text" && frame.data) {
            frames.push(JSON.parse(frame.data));
          }
        }
      },
    });
    const subId = "live-readiness-regression";
    const check = (kind: number | undefined = 9, exactChannel = true) =>
      ready({ channelName: "general", kind, exactChannel });
    const send = (message: unknown[]) =>
      invoke("plugin:websocket|send", {
        id,
        message: { type: "Text", data: JSON.stringify(message) },
      });
    const req = async (filters: Array<Record<string, unknown>>) => {
      frames.length = 0;
      await send(["REQ", subId, ...filters]);
      if (!frames.some((frame) => frame[0] === "EOSE" && frame[1] === subId)) {
        throw new Error(`REQ was not accepted: ${JSON.stringify(frames)}`);
      }
      return check();
    };

    try {
      const before = check();
      const splitGlobal = await req([
        { "#h": [channel], kinds: [30078] },
        { kinds: [9] },
      ]);
      const splitOtherChannel = await req([
        { "#h": [channel], kinds: [30078] },
        { "#h": ["other"], kinds: [9] },
      ]);
      const sameFilter = await req([{ "#h": [channel], kinds: [9] }]);
      const emptyKinds = await req([
        { "#h": [channel], "#p": [owner], kinds: [] },
      ]);
      const omittedKinds = await req([{ "#h": [channel], "#p": [owner] }]);
      const globalOnly = await req([{ kinds: [9] }]);
      const legacyGlobal = check(9, false);
      await send(["CLOSE", subId]);
      const afterClose = check();
      return {
        before,
        splitGlobal,
        splitOtherChannel,
        sameFilter,
        emptyKinds,
        omittedKinds,
        globalOnly,
        legacyGlobal,
        afterClose,
      };
    } finally {
      await invoke("plugin:websocket|disconnect", { id });
    }
  });

  expect(observed).toEqual({
    before: false,
    splitGlobal: false,
    splitOtherChannel: false,
    sameFilter: true,
    emptyKinds: false,
    omittedKinds: true,
    globalOnly: false,
    legacyGlobal: true,
    afterClose: false,
  });
});
