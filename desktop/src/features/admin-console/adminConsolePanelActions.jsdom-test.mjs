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
  mountCommunityPanel,
  settle,
  capturedToasts,
  capturedErrorToasts,
  ipcHandlers,
  TEST_COMMUNITY,
  CM_ORIGIN,
  CM_PUBKEY,
  TEST_RELAY_WS_URL,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";
import { pubkeyToNpub } from "@/shared/lib/nostrUtils.ts";

afterEach(resetTestState);

const TARGET = "ab".repeat(32);
const HOST = TEST_COMMUNITY.host;
const BETA = {
  id: "22222222-2222-4222-8222-222222222222",
  host: "beta.example.com",
  icon: null,
};
const q = (c, id) => c.querySelector(`[data-testid='${id}']`);

const memberDto = (pubkey, extra = {}) => ({
  pubkey,
  profile: null,
  role: "member",
  banned: false,
  mutedUntil: null,
  isStaff: false,
  ...extra,
});

const readReject = (
  relayStatus,
  { code = null, bodyEmpty = false, bodyComplete = true } = {},
) =>
  Promise.reject({
    message: code ? `admin API error: ${code}` : "admin API error: ",
    relayStatus,
    bodyComplete,
    bodyEmpty,
    code,
  });

/** Default community reads: a plain member, no events, empty search. */
function stubReads() {
  for (const [cmd, fn] of [
    ["admin_get_member", ({ pubkey }) => Promise.resolve(memberDto(pubkey))],
    ["admin_get_event", () => readReject(404, { code: "event_not_found" })],
    ["admin_search_members", () => Promise.resolve({ items: [] })],
    ["admin_list_feedback", () => Promise.resolve([])],
    [
      "admin_list_communities",
      () =>
        Promise.resolve({ items: [TEST_COMMUNITY, BETA], nextCursor: null }),
    ],
  ]) {
    if (!ipcHandlers.get(cmd)) setIpcHandler(cmd, fn);
  }
}

async function mountActions({ canMutate = true } = {}) {
  stubReads();
  const panel = mountCommunityPanel(CM_ORIGIN, CM_PUBKEY, "actions", {
    canMutate,
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

/** Open `community`'s page from the Communities tab, on its Actions section. */
async function openActions(c, community = TEST_COMMUNITY) {
  await click(c, "admin-tab-communities");
  await settle();
  await click(c, `community-row-${community.host}`);
  await click(c, "community-section-actions");
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
    await click(c, "direct-action-delete");
    await type(c, "direct-target-input", "nevent1notreal");
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-error").textContent, /64-hex event id/);
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

test("actions-success: confirm sends the frozen intent with signer, relay, and the page host", async () => {
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
      communityHost: HOST,
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

test("actions-identity: a signer change remounts the controller and drops the frozen intent", async () => {
  // Mutation: remove the DirectActionsProvider key → frozen intent survives → RED.
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
    await openActions(c);
    assert.ok(!q(c, "direct-confirm"), "direct-confirm must be absent");
    assert.equal(q(c, "direct-member-input").value, "");
    assert.equal(calls, 0);
  } finally {
    await unmount();
  }
});

test("actions-disabled-auth: with admin auth disabled, community pages render read-only", async () => {
  // Mutation: drop `!canMutate` from any community-page control's disabled
  // gate, or restore a Communities-tab gate on canMutate → RED.
  const banned = "29".repeat(32);
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({
      items: [
        {
          pubkey: banned,
          banned: true,
          banExpiresAt: null,
          banReason: "spam",
          mutedUntil: "2099-01-01T00:00:00Z",
          muteReason: "noise",
          actorPubkey: "aa".repeat(32),
          updatedAt: "2024-06-01T09:00:00Z",
        },
      ],
      nextCursor: null,
    }),
  );
  const { container: c, unmount } = await mountActions({ canMutate: false });
  try {
    await openActions(c);
    assert.ok(q(c, "community-page"), "community page renders");
    for (const id of [
      "direct-action-ban",
      "direct-action-timeout",
      "direct-action-delete",
      "direct-member-input",
      "direct-reason-input",
      "direct-review-btn",
    ]) {
      assert.ok(q(c, id)?.disabled, `${id} must be disabled`);
    }
    await click(c, "community-section-members");
    await pickKey(c, TARGET);
    for (const id of ["member-ban", "member-timeout"]) {
      assert.ok(q(c, id)?.disabled, `${id} must be disabled`);
    }
    await click(c, "community-section-restrictions");
    await settle(50);
    for (const id of [
      `restrictions-lift-ban-btn-${banned}`,
      `restrictions-lift-timeout-btn-${banned}`,
    ]) {
      assert.ok(q(c, id)?.disabled, `${id} must be disabled`);
    }
  } finally {
    await unmount();
  }
});

test("actions-review-race: a late second Review never replaces the submitted intent", async () => {
  // Mutation: drop the in-flight guard in DirectActionsProvider.locked() → RED
  // (the second Review re-freezes with a new requestId and Retry sends it).
  // Relay reads answer at once while the page mounts, then are held from
  // the first Review on so the two Reviews race.
  const relays = [];
  let hold = false;
  setIpcHandler("get_relay_ws_url", () =>
    hold
      ? new Promise((resolve) => relays.push(() => resolve(TEST_RELAY_WS_URL)))
      : Promise.resolve(TEST_RELAY_WS_URL),
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
    hold = true;
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

test("actions-page-host: the form acts in the page's community and has no host field", async () => {
  // Mutation: send the connected host instead of the page's → RED.
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve("relay.test"),
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
    assert.ok(!q(c, "direct-host-input"), "no free-text host");
    assert.match(q(c, "community-banner").textContent, /alpha\.example\.com/);
    await pickKey(c, TARGET);
    await click(c, "direct-review-btn");
    assert.match(q(c, "direct-confirm").textContent, /alpha\.example\.com/);
    await click(c, "direct-confirm-btn");
    assert.equal(sent[0].communityHost, HOST);
  } finally {
    await unmount();
  }
});

test("actions-member-search: a name result is sent as hex and named on the confirm step", async () => {
  const searched = [];
  setIpcHandler("admin_search_members", (args) => {
    searched.push(args);
    return Promise.resolve({
      items: args.q.startsWith("ali")
        ? [
            {
              pubkey: TARGET,
              displayName: "Alice",
              nip05: null,
              avatarUrl: null,
            },
          ]
        : [],
    });
  });
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
    assert.deepEqual(searched.at(-1), {
      origin: CM_ORIGIN,
      communityHost: HOST,
      q: "ali",
    });
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

const OTHER = "cd".repeat(32);

function searchReturns(users) {
  setIpcHandler("admin_search_members", () =>
    Promise.resolve({
      items: users.map(([pubkey, name]) => ({
        pubkey,
        displayName: name,
        nip05: null,
        avatarUrl: null,
      })),
    }),
  );
}

test("actions-community-change-drops-member: a name picked in one community isn't carried to another", async () => {
  // Mutation: key the draft by nothing instead of by host → RED.
  searchReturns([[TARGET, "Alice"]]);
  const { container: c, unmount } = await mountActions();
  try {
    await type(c, "direct-member-input", "ali");
    await settle();
    await click(c, `direct-member-result-${TARGET}`);
    assert.ok(q(c, "direct-member-selected"));
    await openActions(c, BETA);
    assert.ok(
      !q(c, "direct-member-selected"),
      "no pick in the other community",
    );
    await click(c, "direct-review-btn");
    assert.ok(!q(c, "direct-confirm"), "nothing to review");
    assert.match(q(c, "direct-error").textContent, /Choose a member/);
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
    assert.ok(q(c, "direct-member-npub"), "selected chip shows its key");
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

test("actions-secret-key: a pasted secret key or key backup is never searched and is flagged", async () => {
  // Mutation: drop the containsSecretKey gate from the picker's search → RED.
  // The backup prefix is assembled so this file stays outside the frontend
  // key-backup source scan's allowlist; it is only ever typed into the picker.
  const backup = ["ncrypt", "sec1"].join("");
  const queries = [];
  setIpcHandler("admin_search_members", ({ q: query }) => {
    queries.push(query);
    return Promise.resolve({ items: [] });
  });
  const { container: c, unmount } = await mountActions();
  try {
    for (const key of [
      `nsec1${"q".repeat(58)}`,
      `NSEC1${"Q".repeat(58)}`,
      `${backup}${"q".repeat(40)}`,
      `${backup.toUpperCase()}${"Q".repeat(40)}`,
      `ban alice ${backup}${"q".repeat(40)} please`,
      `see:${backup.toUpperCase()}${"Q".repeat(40)}`,
    ]) {
      await type(c, "direct-member-input", key);
      await settle();
      assert.match(
        q(c, "direct-member-secret")?.textContent ?? "",
        /secret key/,
        `no warning for ${key.slice(0, 10)}`,
      );
    }
    assert.deepEqual(queries, [], "a secret key reached admin_search_members");
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
  if (tab === "actions") await openActions(c);
  else await click(c, `admin-tab-${tab}`);
}

test("actions-tab-roundtrip-pending: a 202 survives a tab switch and Retry reuses the requestId", async () => {
  // Mutation: mount DirectActionsProvider inside ActionsSection → RED.
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
    assert.equal(capturedErrorToasts.length, 1, "a hidden failure toasts");
    assert.match(
      capturedErrorToasts[0],
      /alpha\.example\.com failed: network down/,
    );
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
  // Mutation: match the message text instead of bodyEmpty → RED.
  const unsupported = "This relay doesn't support direct actions yet.";
  for (const [status, message, empty, expected] of [
    [404, "admin API error: ", true, unsupported],
    [405, "admin API error: ", true, unsupported],
    [404, "admin API error: Not Found", false, "admin API error: Not Found"],
    [405, "admin API error: ", false, "admin API error: "],
  ]) {
    setIpcHandler("admin_direct_action", () =>
      mutationReject(message, status, true, empty),
    );
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

// ── Multi-community: navigation, fencing, lookups ──────────────────────────

const EVENT = "ef".repeat(32);
const preview = (content) => ({
  id: EVENT,
  authorPubkey: TARGET,
  kind: 9,
  content,
  createdAt: "2026-09-30T00:00:00Z",
  deletedAt: null,
  channelId: null,
});

test("actions-pending-nav: a pending intent survives a community change and only Discard drops it", async () => {
  // Mutation: drop the frozen-elsewhere guard in ActionsSection → RED (B's
  // form offers Review while A's intent is pending).
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
    await openActions(c, BETA);
    assert.ok(q(c, "direct-elsewhere"), "B shows A's pending intent");
    assert.match(q(c, "direct-elsewhere").textContent, /alpha\.example\.com/);
    assert.ok(!q(c, "direct-review-btn"), "B can't review while A is pending");
    assert.ok(!q(c, "direct-confirm-btn"), "A's intent is read-only on B");
    await openActions(c);
    await click(c, "direct-confirm-btn");
    assert.equal(ids.length, 2);
    assert.equal(ids[0], ids[1], "the retry after the trip reuses the id");
    await openActions(c, BETA);
    await click(c, "direct-discard-btn");
    assert.ok(!q(c, "direct-elsewhere"));
    assert.ok(q(c, "direct-review-btn"), "Discard frees B's form");
  } finally {
    await unmount();
  }
});

test("actions-fenced-lookup: a late member lookup from one community never unlocks another", async () => {
  // Mutation: drop communityHost from the lookup fence key → RED.
  const lookups = {};
  setIpcHandler(
    "admin_get_member",
    ({ communityHost }) =>
      new Promise((resolve) => {
        lookups[communityHost] = resolve;
      }),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await pickKey(c, TARGET);
    await openActions(c, BETA);
    await pickKey(c, TARGET);
    assert.ok(q(c, "direct-review-btn").disabled, "B waits for its lookup");
    await act(async () => lookups[HOST](memberDto(TARGET)));
    await settle();
    assert.ok(q(c, "direct-review-btn").disabled, "A's answer can't unlock B");
    await act(async () =>
      lookups[BETA.host](memberDto(TARGET, { role: null, banned: true })),
    );
    await settle();
    assert.match(
      q(c, "direct-member-state").textContent,
      /Not on the community roster · Currently banned/,
    );
    assert.ok(!q(c, "direct-review-btn").disabled);
  } finally {
    await unmount();
  }
});

test("actions-same-ids: one pubkey and one event id read per community", async () => {
  // Mutation: read the lookups with the connected host instead of the page's → RED.
  setIpcHandler("admin_get_member", ({ communityHost, pubkey }) =>
    Promise.resolve(
      memberDto(pubkey, {
        isStaff: false,
        banned: communityHost === BETA.host,
      }),
    ),
  );
  setIpcHandler("admin_get_event", ({ communityHost }) =>
    communityHost === HOST
      ? Promise.resolve(preview("content in a"))
      : readReject(404, { code: "event_not_found" }),
  );
  const { container: c, unmount } = await mountActions();
  try {
    await pickKey(c, TARGET);
    assert.doesNotMatch(q(c, "direct-member-state").textContent, /banned/);
    await click(c, "direct-action-delete");
    await type(c, "direct-target-input", EVENT);
    await settle();
    assert.match(q(c, "direct-event-preview").textContent, /content in a/);
    await openActions(c, BETA);
    await pickKey(c, TARGET);
    assert.match(q(c, "direct-member-state").textContent, /Currently banned/);
    await click(c, "direct-action-delete");
    await type(c, "direct-target-input", EVENT);
    await settle();
    assert.ok(!q(c, "direct-event-preview"), "A's message never shows in B");
    assert.equal(
      q(c, "direct-lookup-error").textContent,
      "Not found in this community.",
    );
    assert.ok(q(c, "direct-review-btn").disabled);
  } finally {
    await unmount();
  }
});

test("actions-preview-errors: only a coded 404 says not found, and no failed preview enables Review", async () => {
  // Mutation: treat any 404 as event_not_found → RED on the empty 404.
  for (const [reply, copy] of [
    [
      () => readReject(404, { code: "event_not_found" }),
      /^Not found in this community\.$/,
    ],
    [
      () => readReject(404, { bodyEmpty: true }),
      /doesn't support community browsing/,
    ],
    [
      () => readReject(405, { bodyEmpty: true }),
      /doesn't support community browsing/,
    ],
    [
      () => readReject(404, { code: "event_not_found", bodyComplete: false }),
      /^(?!Not found)/,
    ],
    [() => readReject(null, { bodyComplete: false }), /^(?!Not found)/],
  ]) {
    setIpcHandler("admin_get_event", reply);
    const { container: c, unmount } = await mountActions();
    try {
      await click(c, "direct-action-delete");
      await type(c, "direct-target-input", EVENT);
      await settle();
      assert.match(q(c, "direct-lookup-error").textContent, copy);
      assert.ok(q(c, "direct-review-btn").disabled, "Review stays off");
    } finally {
      await unmount();
    }
  }
});

test("actions-delete-link: a note link previews the message and sends its hex id", async () => {
  // Mutation: accept only hex in parseEventIdInput → RED.
  const { noteEncode } = await import("nostr-tools/nip19");
  setIpcHandler("admin_get_event", ({ id }) =>
    Promise.resolve(preview(`preview of ${id.slice(0, 4)}`)),
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
    await click(c, "direct-action-delete");
    await type(c, "direct-target-input", noteEncode(EVENT));
    await settle();
    assert.match(q(c, "direct-event-preview").textContent, /preview of efef/);
    await click(c, "direct-review-btn");
    assert.ok(
      q(c, "direct-confirm").querySelector(
        "[data-testid='direct-event-preview']",
      ),
    );
    await click(c, "direct-confirm-btn");
    assert.equal(sent[0].target, EVENT);
  } finally {
    await unmount();
  }
});

test("actions-staff-target: a staff member can't reach Review", async () => {
  // Mutation: drop the isStaff check from lookupBlock → RED.
  setIpcHandler("admin_get_member", ({ pubkey }) =>
    Promise.resolve(memberDto(pubkey, { isStaff: true })),
  );
  let calls = 0;
  setIpcHandler("admin_direct_action", () => {
    calls += 1;
    return Promise.resolve({});
  });
  const { container: c, unmount } = await mountActions();
  try {
    await pickKey(c, TARGET);
    assert.match(
      q(c, "direct-lookup-error").textContent,
      /Relay staff can't be banned/,
    );
    assert.ok(q(c, "direct-review-btn").disabled);
    assert.equal(calls, 0);
  } finally {
    await unmount();
  }
});

test("fenced-load: an answer for an old key never shows under the new key", async () => {
  // Mutation: drop both the effect-local `active` flag and the result-key
  // check from useFencedLoad → RED (A's late answer shows under key B).
  const React = await import("react");
  const { createRoot } = await import("react-dom/client");
  const { useFencedLoad } = await import("./AdminConsoleActionsTab.tsx");
  const resolvers = {};
  const seen = [];
  function Probe({ k }) {
    const state = useFencedLoad(
      k,
      () => new Promise((resolve) => (resolvers[k] = resolve)),
    );
    seen.push(`${k}:${state.status}:${state.data ?? ""}`);
    return null;
  }
  const el = document.createElement("div");
  const root = createRoot(el);
  await act(async () =>
    root.render(React.createElement(Probe, { k: "alpha" })),
  );
  await act(async () => root.render(React.createElement(Probe, { k: "beta" })));
  await act(async () => resolvers.alpha("from alpha"));
  assert.equal(seen.at(-1), "beta:loading:", "alpha's answer must not show");
  await act(async () => resolvers.beta("from beta"));
  assert.equal(seen.at(-1), "beta:ok:from beta");
  await act(async () => root.unmount());
});
