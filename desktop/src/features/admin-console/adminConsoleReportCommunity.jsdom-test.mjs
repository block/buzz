/**
 * Community identity on report lists and at report resolution: each group of
 * rows is headed by its community, and confirming a resolution names the
 * report's community and warns when it is not the one the app is connected to.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  act,
  fireEvent,
  makeOpenReportFixtures,
  mountPanel,
  resetTestState,
  setIpcHandler,
  settle,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

const origin = "https://admin.example.com";
const pubkey = "e4".repeat(32);
const ALPHA = "[data-testid='community-badge-alpha.example.com']";

test("report-group-badge: a community group names its community once, in its heading", async () => {
  const { openItem } = makeOpenReportFixtures(
    "00000000-0000-0000-0000-0000000000e1",
  );
  const second = { ...openItem, id: "00000000-0000-0000-0000-0000000000e2" };
  setIpcHandler("admin_list_reports", () =>
    Promise.resolve([openItem, second]),
  );
  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    const group = container.querySelector("[data-testid='community-group']");
    assert.equal(group.querySelectorAll("ul > li").length, 2);
    assert.equal(group.querySelectorAll(ALPHA).length, 1, "one badge");
    assert.ok(
      group.querySelector(`[data-testid='community-group-host'] ${ALPHA}`),
      "the badge heads the group",
    );
  } finally {
    await unmount();
  }
});

test("report-resolve-not-connected: confirming a report from another community shows its badge and the warning", async () => {
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve("beta.example.com"),
  );
  makeOpenReportFixtures("00000000-0000-0000-0000-0000000000e3");
  const { container, doRender, unmount } = mountPanel({ origin, pubkey });
  try {
    await doRender();
    await settle(30);
    const row = container.querySelector(
      "[data-testid='community-group'] ul > li button",
    );
    await act(async () => fireEvent.click(row));
    await settle(30);
    await act(async () =>
      fireEvent.click(
        container.querySelector("[data-testid='action-btn-dismiss']"),
      ),
    );
    await settle();
    const confirm = container.querySelector(
      "[data-testid='resolve-community']",
    );
    assert.ok(confirm, "confirm names the report's community");
    assert.ok(confirm.querySelector(ALPHA));
    assert.ok(confirm.querySelector("[data-testid='community-not-connected']"));
  } finally {
    await unmount();
  }
});
