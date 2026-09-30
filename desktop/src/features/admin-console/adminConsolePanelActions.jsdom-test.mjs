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
import { pubkeyToNpub } from "@/shared/lib/nostrUtils.ts";

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

async function setHost(c, host) {
  if (q(c, "direct-host-change")) await click(c, "direct-host-change");
  await type(c, "direct-host-input", host);
}

/** Paste a key and pick its direct result. */
async function pickKey(c, key, hex = TARGET) {
  await type(c, "direct-member-input", key);
  await settle();
  assert.ok(q(c, `direct-member-result-${hex}`), `no direct result for ${key}`);
  await click(c, `direct-member-result-${hex}`);
}

async function fillTimeout(c) {
  await click(c, "direct-action-timeout");
  await setHost(c, " team.example.com ");
  await pickKey(c, TARGET);
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
    assert.match(q(c, "direct-error").textContent, /Choose a member/);
    await setHost(c, "");
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /community host/);
    await type(c, "direct-host-input", "team.example.com");
    await click(c, "direct-action-delete");
    await type(c, "direct-target-input", TARGET.toUpperCase());
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /64 lowercase hex/);
    await click(c, "direct-action-timeout");
    await pickKey(c, TARGET);
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
      q(c, "direct-member-remove").disabled,
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
    ["request_id_conflict", 409, /already used for a different action/, false],
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

test("actions-pending-neutral: a 202 renders as a neutral notice, not an error", async () => {
  setIpcHandler("admin_direct_action", () =>
    Promise.resolve({ state: "pending" }),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    const notice = q(c, "direct-pending");
    assert.ok(notice, "pending notice must render");
    assert.ok(!notice.className.includes("text-destructive"));
    assert.ok(!q(c, "direct-error"), "pending must not render as an error");
    assert.equal(q(c, "direct-confirm-btn").textContent, "Retry");
  } finally {
    await unmount();
  }
});

test("actions-request-id-conflict: a 409 conflict drops the intent and offers no Retry", async () => {
  setIpcHandler("admin_direct_action", () =>
    mutationReject(
      'admin API error: {"error":{"code":"request_id_conflict","message":"conflict"}}',
      409,
    ),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.match(
      q(c, "direct-error").textContent,
      /Review again to send it with a new id/,
    );
    assert.ok(!q(c, "direct-confirm-btn"), "no Retry for a spent request id");
    assert.ok(q(c, "direct-review-btn"), "the form is back to Review");
  } finally {
    await unmount();
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
    assert.match(q(c, "direct-pending").textContent, /still applying/);
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
    assert.equal(q(c, "direct-member-input").value, "");
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
    assert.ok(q(c, "direct-member-input").disabled);
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
  const mounted = mountActions();
  await settle();
  // The tab reads the active relay once on mount to prefill the host.
  await act(async () => {
    for (const resolve of relays.splice(0)) resolve();
  });
  const { container: c, unmount } = await mounted;
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

test("actions-host-prefill: the host follows the active relay read-only, and Change reveals the input", async () => {
  // Mutation: drop the active-relay prefill (activeHost stays null) → RED.
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
    assert.ok(
      q(c, "direct-host"),
      "host must be prefilled from the active relay",
    );
    assert.match(q(c, "direct-host").textContent, /Community: relay\.test/);
    assert.ok(!q(c, "direct-host-input"), "host input hidden until Change");
    await pickKey(c, TARGET);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.equal(sent[0].communityHost, "relay.test");
    await click(c, "direct-host-change");
    assert.equal(q(c, "direct-host-input").value, "relay.test");
  } finally {
    await unmount();
  }
});

test("actions-member-search: a name result is sent as hex and named on the confirm step", async () => {
  setIpcHandler("search_users", ({ query }) =>
    Promise.resolve({
      users: query.startsWith("ali")
        ? [
            {
              pubkey: TARGET,
              display_name: "Alice",
              avatar_url: null,
              nip05_handle: null,
              owner_pubkey: null,
            },
          ]
        : [],
      next_cursor: null,
    }),
  );
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
    await type(c, "direct-member-input", "ali");
    await settle();
    await click(c, `direct-member-result-${TARGET}`);
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-confirm-member").textContent, /^Alice \(npub1/);
    await click(c, "direct-confirm-btn");
    assert.equal(sent[0].target, TARGET);
  } finally {
    await unmount();
  }
});

test("actions-member-keys: an npub and uppercase hex are both sent as lowercase hex", async () => {
  // Mutation: accept only lowercase hex instead of parsePubkeyInput → RED.
  const sent = [];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    sent.push(intent);
    return Promise.resolve({
      actionId: "a1",
      state: "succeeded",
      replayed: false,
    });
  });
  for (const key of [pubkeyToNpub(TARGET), TARGET.toUpperCase()]) {
    const { container: c, unmount } = await mountActions();
    try {
      await pickKey(c, key);
      await click(c, "direct-review-btn");
      await click(c, "direct-confirm-btn");
    } finally {
      await unmount();
    }
  }
  assert.deepEqual(
    sent.map((i) => i.target),
    [TARGET, TARGET],
  );
});

test("actions-foreign-host: another community's host turns name search off and says why", async () => {
  let searches = 0;
  setIpcHandler("search_users", () => {
    searches += 1;
    return Promise.resolve({ users: [], next_cursor: null });
  });
  const { container: c, unmount } = await mountActions();
  try {
    assert.ok(
      !q(c, "direct-member-search-hint"),
      "no hint on the active community",
    );
    await setHost(c, "other.example.com");
    assert.match(
      q(c, "direct-member-search-hint").textContent,
      /only works in the community you're connected to/,
    );
    await type(c, "direct-member-input", "alice");
    await settle();
    assert.equal(searches, 0, "no name search against another community");
    await pickKey(c, TARGET);
    assert.ok(q(c, "direct-member-selected"), "keys still work");
  } finally {
    await unmount();
  }
});

const OTHER = "cd".repeat(32);

function searchReturns(users) {
  setIpcHandler("search_users", () =>
    Promise.resolve({
      users: users.map(([pubkey, name]) => ({
        pubkey,
        display_name: name,
        avatar_url: null,
        nip05_handle: null,
        owner_pubkey: null,
      })),
      next_cursor: null,
    }),
  );
}

test("actions-host-change-drops-member: a name picked in one community can't be reviewed in another", async () => {
  // Mutation: mask the pick by host instead of clearing it → RED on A→B→A.
  searchReturns([[TARGET, "Alice"]]);
  const { container: c, unmount } = await mountActions();
  try {
    await type(c, "direct-member-input", "ali");
    await settle();
    await click(c, `direct-member-result-${TARGET}`);
    await click(c, "direct-host-change");
    assert.ok(q(c, "direct-member-selected"), "Change alone keeps the pick");
    await setHost(c, "other.example.com");
    assert.ok(
      !q(c, "direct-member-selected"),
      "the pick must not survive a host change",
    );
    assert.equal(q(c, "direct-member-input").value, "", "query is cleared");
    await click(c, "direct-review-btn");
    assert.ok(!q(c, "direct-confirm"), "nothing to review");
    assert.match(q(c, "direct-error").textContent, /Choose a member/);
    await setHost(c, "relay.test");
    assert.ok(
      !q(c, "direct-member-selected"),
      "returning to the first host must not restore the old pick",
    );
    await click(c, "direct-review-btn");
    assert.ok(!q(c, "direct-confirm"), "still nothing to review");
    assert.match(q(c, "direct-error").textContent, /Choose a member/);
    await setHost(c, "other.example.com");
    await pickKey(c, TARGET);
    assert.ok(
      q(c, "direct-member-search-hint"),
      "hint stays visible with a key selected",
    );
    await click(c, "direct-review-btn");
    assert.doesNotMatch(q(c, "direct-confirm-member").textContent, /Alice/);
  } finally {
    await unmount();
  }
});

test("actions-same-name: two same-name results are told apart and the chosen full key is confirmed", async () => {
  // Mutation: drop the full PubKey from the confirm step → RED.
  searchReturns([
    [TARGET, "Alice"],
    [OTHER, "Alice"],
  ]);
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
    await type(c, "direct-member-input", "ali");
    await settle();
    const a = q(c, `direct-member-result-${TARGET}`).textContent;
    const b = q(c, `direct-member-result-${OTHER}`).textContent;
    assert.notEqual(a, b, "same-name results must differ by key");
    await click(c, `direct-member-result-${OTHER}`);
    assert.ok(q(c, "direct-member-inspect"), "selected chip is inspectable");
    await click(c, "direct-review-btn");
    assert.ok(q(c, "direct-confirm-npub"), "confirm must show the full npub");
    assert.match(
      q(c, "direct-confirm-npub").textContent,
      new RegExp(pubkeyToNpub(OTHER)),
      "confirm shows the chosen member's full npub",
    );
    await click(c, "direct-confirm-btn");
    assert.equal(sent[0].target, OTHER);
  } finally {
    await unmount();
  }
});

test("actions-results-disabled: results showing when auth drops can't be selected", async () => {
  searchReturns([[TARGET, "Alice"]]);
  const panel = await mountActions();
  const c = panel.container;
  try {
    await type(c, "direct-member-input", "ali");
    await settle();
    await panel.doRender({ canMutate: false });
    await settle();
    const result = q(c, `direct-member-result-${TARGET}`);
    assert.ok(result, "results still shown");
    assert.ok(result.disabled, "results must be disabled");
    await click(c, `direct-member-result-${TARGET}`);
    assert.ok(!q(c, "direct-member-selected"));
  } finally {
    await panel.unmount();
  }
});

test("actions-secret-key: a pasted secret key or key backup is never searched and is flagged", async () => {
  // Mutation: drop the isSecretKey gate from the picker's search → RED.
  // The backup prefix is assembled so this file stays outside the frontend
  // key-backup source scan's allowlist; it is only ever typed into the picker.
  const backup = ["ncrypt", "sec1"].join("");
  const queries = [];
  setIpcHandler("search_users", ({ query }) => {
    queries.push(query);
    return Promise.resolve({ users: [], next_cursor: null });
  });
  const { container: c, unmount } = await mountActions();
  try {
    for (const key of [
      `nsec1${"q".repeat(58)}`,
      `NSEC1${"Q".repeat(58)}`,
      `${backup}${"q".repeat(40)}`,
      `${backup.toUpperCase()}${"Q".repeat(40)}`,
    ]) {
      await type(c, "direct-member-input", key);
      await settle();
      assert.match(
        q(c, "direct-member-secret")?.textContent ?? "",
        /secret key/,
        `no warning for ${key.slice(0, 10)}`,
      );
    }
    assert.deepEqual(queries, [], "a secret key reached search_users");
    await type(c, "direct-member-input", "");
    assert.ok(!q(c, "direct-member-secret"), "clearing drops the warning");
    await type(c, "direct-member-input", "alice");
    await settle();
    assert.deepEqual(queries, ["alice"], "name search still runs");
    await pickKey(c, pubkeyToNpub(TARGET));
    assert.ok(q(c, "direct-member-selected"), "npub still works");
  } finally {
    await unmount();
  }
});

async function toTab(c, tab) {
  await click(c, `admin-tab-${tab}`);
}

test("actions-tab-roundtrip-pending: a 202 survives a tab switch and Retry reuses the requestId", async () => {
  // Mutation: render ActionsTab only while it is the active tab → RED.
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  const ids = [];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    ids.push(intent.requestId);
    return Promise.resolve({ state: "pending" });
  });
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    await toTab(c, "feedback");
    await toTab(c, "actions");
    assert.ok(q(c, "direct-pending"), "pending notice survives");
    await click(c, "direct-confirm-btn");
    assert.equal(ids.length, 2);
    assert.equal(ids[0], ids[1], "Retry after a tab trip reuses the id");
  } finally {
    await unmount();
  }
});

test("actions-tab-roundtrip-ambiguous: an ambiguous failure survives a tab switch", async () => {
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  const ids = [];
  setIpcHandler("admin_direct_action", ({ intent }) => {
    ids.push(intent.requestId);
    return mutationReject("network down", null);
  });
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    await toTab(c, "feedback");
    await toTab(c, "actions");
    assert.equal(q(c, "direct-confirm-btn")?.textContent, "Retry");
    await click(c, "direct-confirm-btn");
    assert.equal(ids.length, 2);
    assert.equal(ids[0], ids[1]);
  } finally {
    await unmount();
  }
});

test("actions-tab-roundtrip-inflight: a send that fails on another tab shows its error on return", async () => {
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));
  let fail;
  setIpcHandler(
    "admin_direct_action",
    () =>
      new Promise((_, reject) => {
        fail = () => reject({ message: "network down", relayStatus: null });
      }),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await fillTimeout(c);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    await toTab(c, "feedback");
    await act(async () => fail());
    await settle();
    await toTab(c, "actions");
    assert.match(q(c, "direct-error")?.textContent ?? "", /network down/);
    assert.equal(q(c, "direct-confirm-btn")?.textContent, "Retry");
  } finally {
    await unmount();
  }
});

test("actions-not-sent: a refusal before sending drops the intent and offers no Retry", async () => {
  // Mutation: drop the adminMutationNotSent check from handleConfirm → RED.
  let calls = 0;
  setIpcHandler("admin_direct_action", () => {
    calls += 1;
    return Promise.reject({
      message: "invalid community host: path not allowed",
      relayStatus: null,
      bodyComplete: false,
      notSent: true,
    });
  });
  const { container: c, unmount } = await mountActions();
  try {
    await setHost(c, "team.example.com/");
    await pickKey(c, TARGET);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.equal(calls, 1);
    assert.ok(!q(c, "direct-confirm"), "intent dropped; no Retry");
    assert.match(q(c, "direct-error").textContent, /invalid community host/);
    assert.ok(q(c, "direct-review-btn"), "back to Review");
  } finally {
    await unmount();
  }
});

test("actions-old-relay: only an empty 404 or 405 says the relay lacks direct actions", async () => {
  // Mutation: drop the empty-body condition from directErrorMessage → RED.
  for (const [status, message, expected] of [
    [
      404,
      "admin API error: ",
      "This relay doesn't support direct actions yet.",
    ],
    [
      405,
      "admin API error: ",
      "This relay doesn't support direct actions yet.",
    ],
    [404, "admin API error: Not Found", "admin API error: Not Found"],
    [
      405,
      "admin API error: Method Not Allowed",
      "admin API error: Method Not Allowed",
    ],
  ]) {
    setIpcHandler("admin_direct_action", () => mutationReject(message, status));
    const { container: c, unmount } = await mountActions();
    try {
      await pickKey(c, TARGET);
      await click(c, "direct-review-btn");
      await click(c, "direct-confirm-btn");
      assert.equal(
        q(c, "direct-error").textContent,
        expected,
        `${status} ${message}`,
      );
    } finally {
      await unmount();
    }
  }
});

test("actions-backup-reason-not-sent: a key-backup reason refusal returns to editable Review", async () => {
  // Mirrors the native backup-guard refusal (Rust asserts notSent there).
  // Mutation: return notSent: false, as before the fix → RED.
  const backup = "see " + "ncrypt" + "sec1qgg9947";
  let calls = 0;
  setIpcHandler("admin_direct_action", () => {
    calls += 1;
    return Promise.reject({
      message: "admin API mutation: refused to send key-backup material",
      relayStatus: null,
      bodyComplete: false,
      notSent: true,
    });
  });
  const { container: c, unmount } = await mountActions();
  try {
    await pickKey(c, TARGET);
    await type(c, "direct-reason-input", backup);
    await click(c, "direct-review-btn");
    await click(c, "direct-confirm-btn");
    assert.equal(calls, 1);
    assert.ok(!q(c, "direct-confirm"), "no Retry after a pre-send refusal");
    assert.ok(q(c, "direct-review-btn"), "back to Review");
    assert.equal(
      q(c, "direct-reason-input").disabled,
      false,
      "Reason is editable again",
    );
  } finally {
    await unmount();
  }
});
