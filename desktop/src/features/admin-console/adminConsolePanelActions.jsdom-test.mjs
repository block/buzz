/**
 * Actions tab tests: validation, frozen intent (host only as the typed query
 * field, same requestId on every retry, signer + relay captured at review),
 * relay error mapping, and the disabled-auth gate.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  fireEvent,
  act,
  setIpcHandler,
  resetTestState,
  mutationReject,
  mountPanel,
  settle,
  capturedToasts,
  CM_ORIGIN,
  CM_PUBKEY,
  TEST_RELAY_WS_URL,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

const TARGET = "ab".repeat(32);
const q = (c, id) => c.querySelector(`[data-testid='${id}']`);

async function mountActions({ canMutate = true } = {}) {
  const panel = mountPanel({
    origin: CM_ORIGIN,
    pubkey: CM_PUBKEY,
    canMutate,
    initialTab: "actions",
  });
  await panel.doRender();
  await settle();
  return panel;
}

async function type(c, id, value) {
  await act(async () => {
    fireEvent.change(q(c, id), { target: { value } });
  });
}

async function click(c, id) {
  await act(async () => {
    fireEvent.click(q(c, id));
  });
  await settle();
}

async function fillTimeout(c) {
  await click(c, "direct-action-timeout");
  await type(c, "direct-host-input", " team.example.com ");
  await type(c, "direct-target-input", TARGET);
  await type(c, "direct-duration-input", "60");
  await type(c, "direct-reason-input", "spam");
}

test("actions-validation: bad target or duration shows an error and sends nothing", async () => {
  let calls = 0;
  setIpcHandler("admin_direct_action", () => {
    calls += 1;
    return Promise.resolve({});
  });
  const { container: c, unmount } = await mountActions();
  try {
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /community host/);
    await type(c, "direct-host-input", "team.example.com");
    await type(c, "direct-target-input", TARGET.toUpperCase());
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /64 lowercase hex/);
    await click(c, "direct-action-timeout");
    await type(c, "direct-target-input", TARGET);
    await type(c, "direct-duration-input", "0");
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /Duration/);
    assert.ok(!q(c, "direct-confirm"), "direct-confirm must be absent");
    assert.equal(calls, 0);
  } finally {
    await unmount();
  }
});

test("actions-success: confirm sends the frozen intent with signer, relay, and typed host", async () => {
  const sent = [];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    sent.push(intent);
    return Promise.resolve({
      actionId: "a1",
      state: "succeeded",
      replayed: false,
    });
  });
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    assert.equal(sent.length, 0, "review alone must not send");
    await click(c, "direct-confirm-btn");
    assert.equal(sent.length, 1);
    const { requestId, ...rest } = sent[0];
    assert.match(requestId, /^[0-9a-f-]{36}$/);
    assert.deepEqual(rest, {
      origin: CM_ORIGIN,
      expectedRelay: TEST_RELAY_WS_URL,
      expectedPubkey: CM_PUBKEY,
      communityHost: "team.example.com",
      action: "timeout",
      target: TARGET,
      reason: "spam",
      expirationSecs: 60,
    });
    assert.deepEqual(capturedToasts, ["Time out member: done"]);
    assert.ok(!q(c, "direct-confirm"), "direct-confirm must be absent");
  } finally {
    await unmount();
  }
});

test("actions-retry: an ambiguous failure keeps the intent and a manual retry reuses the requestId", async () => {
  // Mutation: mint requestId in handleConfirm, or drop preserveRequestIdOnError → RED.
  const ids = [];
  const replies = [
    () => mutationReject("network down", null),
    () =>
      mutationReject(
        'admin API error: {"error":{"code":"internal","message":"boom"}}',
        500,
      ),
    () =>
      Promise.resolve({ actionId: "a1", state: "succeeded", replayed: true }),
  ];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    ids.push(intent.requestId);
    return replies[ids.length - 1]();
  });
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.equal(q(c, "direct-confirm-btn").textContent, "Retry");
    assert.ok(
      q(c, "direct-target-input").disabled,
      "fields stay locked to the frozen intent",
    );
    await click(c, "direct-confirm-btn");
    await click(c, "direct-confirm-btn");
    assert.equal(ids.length, 3);
    assert.equal(
      new Set(ids).size,
      1,
      "every retry carries the same requestId",
    );
  } finally {
    await unmount();
  }
});

test("actions-errors: relay codes map to copy; a definitive 4xx unlocks the form", async () => {
  const cases = [
    ["target_is_staff", 409, /Relay staff can't be banned/, true],
    ["request_id_conflict", 409, /already used for a different action/, true],
    ["event_not_in_community", 404, /not in that community/, false],
    ["unknown_community_host", 400, /unknown host/, false],
    ["enforcement_failed", 422, /enforcement broke/, false],
  ];
  for (const [code, status, copy, keepsIntent] of cases) {
    const message = copy.source.replace(/\\/g, "");
    setIpcHandler("admin_direct_action", () =>
      mutationReject(
        `admin API error: {"error":{"code":"${code}","message":"${message}"}}`,
        status,
      ),
    );
    const { container: c, unmount } = await mountActions();
    try {
      await fillTimeout(c);
      await click(c, "direct-review-btn");
      await click(c, "direct-confirm-btn");
      assert.match(q(c, "direct-error").textContent, copy, code);
      assert.equal(Boolean(q(c, "direct-confirm")), keepsIntent, code);
    } finally {
      await unmount();
    }
  }
});

test("actions-pending: a 202 keeps the intent for a same-id retry", async () => {
  setIpcHandler("admin_direct_action", () =>
    Promise.resolve({ state: "pending" }),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.match(q(c, "direct-error").textContent, /still applying/);
    assert.ok(q(c, "direct-confirm"));
    assert.deepEqual(capturedToasts, []);
  } finally {
    await unmount();
  }
});

test("actions-identity: a signer change remounts the tab and drops the frozen intent", async () => {
  // Mutation: remove the ActionsTab key → frozen intent survives → RED.
  let calls = 0;
  setIpcHandler("admin_direct_action", () => {
    calls += 1;
    return Promise.resolve({
      actionId: "a1",
      state: "succeeded",
      replayed: false,
    });
  });
  const { container: c, doRender, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    assert.ok(q(c, "direct-confirm"));
    await doRender({ origin: CM_ORIGIN, pubkey: "ee".repeat(32) });
    await settle();
    assert.ok(!q(c, "direct-confirm"), "direct-confirm must be absent");
    assert.equal(q(c, "direct-target-input").value, "");
    assert.equal(calls, 0);
  } finally {
    await unmount();
  }
});

test("actions-disabled-auth: canMutate=false keeps Review and every field off", async () => {
  // Mutation: drop !canMutate from the Review/lock gates → RED.
  const { container: c, unmount } = await mountActions({ canMutate: false });
  try {
    assert.ok(q(c, "direct-review-btn").disabled);
    assert.ok(q(c, "direct-target-input").disabled);
    assert.ok(q(c, "direct-action-ban").disabled);
  } finally {
    await unmount();
  }
});

test("actions-review-race: a late second Review never replaces the submitted intent", async () => {
  // Mutation: drop the in-flight guard at the top of handleReview → RED
  // (the second Review re-freezes with a new requestId and Retry sends it).
  const relays = [];
  setIpcHandler(
    "get_relay_ws_url",
    () =>
      new Promise((resolve) => relays.push(() => resolve(TEST_RELAY_WS_URL))),
  );
  const ids = [];
  const replies = [
    () => mutationReject("network down", null),
    () =>
      Promise.resolve({ actionId: "a1", state: "succeeded", replayed: true }),
  ];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    ids.push(intent.requestId);
    return replies[ids.length - 1]();
  });
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await act(async () => {
      fireEvent.click(q(c, "direct-review-btn"));
      fireEvent.click(q(c, "direct-review-btn"));
    });
    const pending = relays.splice(0);
    await act(async () => pending[0]());
    await settle();
    await click(c, "direct-confirm-btn");
    await act(async () => {
      for (const resolve of pending.slice(1)) resolve();
    });
    await settle();
    await click(c, "direct-confirm-btn");
    assert.equal(ids.length, 2);
    assert.equal(ids[0], ids[1], "retry must replay the submitted requestId");
  } finally {
    await unmount();
  }
});

test("actions-audience: the reason's recipients are disclosed and the frozen reason is shown", async () => {
  // Mutation: remove the direct-reason-audience line or the confirm reason → RED.
  const { container: c, unmount } = await mountActions();
  try {
    const audience = () => q(c, "direct-reason-audience").textContent;
    assert.equal(audience(), "Sent verbatim to the affected user.");
    await click(c, "direct-action-delete");
    assert.equal(
      audience(),
      "Sent verbatim to the affected user and posted publicly in the room.",
    );
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    assert.equal(audience(), "Sent verbatim to the affected user.");
    assert.equal(q(c, "direct-confirm-reason").textContent, "Reason: spam");
  } finally {
    await unmount();
  }
});
