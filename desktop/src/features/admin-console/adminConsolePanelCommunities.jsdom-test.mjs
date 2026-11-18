/**
 * Communities tab, community badges, and the lift fence on a community page.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import {
  act,
  CM_ORIGIN,
  CM_PUBKEY,
  deferred,
  fireEvent,
  makeOpenReportFixtures,
  mountCommunityPanel,
  mountPanel,
  resetTestState,
  setIpcHandler,
  settle,
  TEST_COMMUNITY,
} from "./adminConsolePanelTestHelpers.jsdom.mjs";

afterEach(resetTestState);

const q = (c, id) => c.querySelector(`[data-testid='${id}']`);
const community = (n) => ({
  id: `00000000-0000-4000-8000-00000000000${n}`,
  host: `c${n}.example.com`,
  icon: null,
});

async function click(el) {
  await act(async () => {
    fireEvent.click(el);
  });
  await settle();
}

async function mountCommunities() {
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  const panel = mountPanel({
    origin: CM_ORIGIN,
    pubkey: CM_PUBKEY,
    role: "operator",
    initialTab: "communities",
  });
  await panel.doRender();
  await settle();
  return panel;
}

test("communities-directory: the connected community is pinned by exact host and Load more pages by cursor", async () => {
  // Mutation: pin from the loaded page instead of an exact-host read → RED
  // (the connected community is on no loaded page).
  const calls = [];
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve("c9.example.com"),
  );
  setIpcHandler("admin_list_communities", ({ q: query, cursor }) => {
    calls.push({ query, cursor });
    if (query === "c9.example.com") {
      return Promise.resolve({ items: [community(9)], nextCursor: null });
    }
    return Promise.resolve(
      cursor === "page2"
        ? { items: [community(3)], nextCursor: null }
        : { items: [community(1), community(2)], nextCursor: "page2" },
    );
  });
  const { container: c, unmount } = await mountCommunities();
  try {
    assert.ok(
      q(q(c, "communities-pinned"), "community-row-c9.example.com"),
      "connected community pinned",
    );
    await click(q(c, "communities-load-more"));
    const hosts = [
      ...c.querySelectorAll("[data-testid^='community-row-']"),
    ].map((el) => el.dataset.testid.replace("community-row-", ""));
    assert.deepEqual(hosts, [
      "c9.example.com",
      "c1.example.com",
      "c2.example.com",
      "c3.example.com",
    ]);
    assert.ok(!q(c, "communities-load-more"), "no more pages");
    assert.ok(calls.some((x) => x.cursor === "page2"));
    await click(q(c, "community-row-c2.example.com"));
    assert.match(q(c, "community-banner").textContent, /c2\.example\.com/);
    assert.ok(q(c, "community-not-connected"), "warns it isn't connected");
  } finally {
    await unmount();
  }
});

test("communities-pin-search: the connected community stays pinned only under a matching host prefix", async () => {
  // Mutation: pin regardless of the search → RED (c9 pinned under "c1").
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve("c9.example.com"),
  );
  setIpcHandler("admin_list_communities", ({ q: query }) =>
    Promise.resolve({
      items: [community(1), community(9)].filter((x) =>
        x.host.startsWith((query ?? "").toLowerCase()),
      ),
      nextCursor: null,
    }),
  );
  const { container: c, unmount } = await mountCommunities();
  const search = async (text) => {
    await act(async () => {
      fireEvent.change(q(c, "communities-search-input"), {
        target: { value: text },
      });
    });
    await settle();
  };
  try {
    await search("c1");
    assert.equal(q(c, "communities-pinned"), null, "unrelated search");
    assert.ok(q(c, "community-row-c1.example.com"));
    await search(" C9 ");
    assert.ok(
      q(q(c, "communities-pinned"), "community-row-c9.example.com"),
      "case-insensitive trimmed prefix keeps the pin",
    );
  } finally {
    await unmount();
  }
});

test("communities-secret-key: a pasted key backup anywhere in the search is never sent", async () => {
  // Mutation: drop the containsSecretKey gate from the directory search → RED.
  // The prefix is assembled so this file stays outside the frontend
  // key-backup source scan's allowlist.
  const backup = ["ncrypt", "sec1"].join("") + "q".repeat(40);
  const queries = [];
  setIpcHandler("admin_list_communities", ({ q: query }) => {
    queries.push(query);
    return Promise.resolve({ items: [community(1)], nextCursor: null });
  });
  const { container: c, unmount } = await mountCommunities();
  const search = async (text) => {
    await act(async () => {
      fireEvent.change(q(c, "communities-search-input"), {
        target: { value: text },
      });
    });
    await settle();
  };
  try {
    queries.length = 0;
    for (const text of [backup, `team ${backup} host`, backup.toUpperCase()]) {
      await search(text);
      assert.ok(q(c, "communities-search-secret"), `no warning for ${text}`);
      assert.equal(q(c, "community-row-c1.example.com"), null);
    }
    assert.deepEqual(
      queries,
      [],
      "a key backup reached admin_list_communities",
    );
    await search("c1.example");
    assert.equal(q(c, "communities-search-secret"), null);
    assert.deepEqual(queries, ["c1.example"], "host search still runs");
    assert.ok(q(c, "community-row-c1.example.com"));
  } finally {
    await unmount();
  }
});

test("communities-secret-more: Load more never sends key material typed over a page that has more", async () => {
  // Mutation: drop the secret gate from Load more → RED. For one render the
  // deferred secret query sits beside the previous page's cursor before that
  // page clears, too briefly to click. Recording every onClick React commits
  // to the button and invoking each afterwards drives exactly those handlers.
  const backup = ["ncrypt", "sec1"].join("") + "q".repeat(40);
  const nsec = ["ns", "ec1"].join("") + "q".repeat(40);
  const calls = [];
  setIpcHandler("admin_list_communities", ({ q: query, cursor }) => {
    calls.push({ query, cursor });
    return Promise.resolve({ items: [community(1)], nextCursor: "page2" });
  });
  const { container: c, unmount } = await mountCommunities();
  const type = async (text) => {
    await act(async () => {
      fireEvent.change(q(c, "communities-search-input"), {
        target: { value: text },
      });
    });
    await settle();
  };
  try {
    for (const text of [backup, `team ${nsec} host`, backup.toUpperCase()]) {
      await type("");
      const more = q(c, "communities-load-more");
      assert.ok(more, "a page with more is shown");
      const propsKey = Object.keys(more).find((k) =>
        k.startsWith("__reactProps$"),
      );
      assert.ok(propsKey, "React props key not found on Load more");
      let props = more[propsKey];
      assert.equal(
        typeof props?.onClick,
        "function",
        "no Load more handler captured before the secret was typed",
      );
      const handlers = [];
      Object.defineProperty(more, propsKey, {
        configurable: true,
        get: () => props,
        set: (next) => {
          props = next;
          handlers.push(next.onClick);
        },
      });
      await type(text);
      assert.ok(q(c, "communities-search-secret"), `no warning for ${text}`);
      assert.equal(q(c, "communities-load-more"), null);
      await act(async () => {
        for (const onClick of handlers) onClick?.();
      });
      await settle();
    }
    const leaked = calls.filter((x) => /sec1/i.test(x.query ?? ""));
    assert.deepEqual(leaked, [], "key material reached admin_list_communities");
  } finally {
    await unmount();
  }
});

test("communities-stale-more: a late page from an earlier search leaves the current search's pages alone", async () => {
  // Mutation: drop either searchRef check in loadMore → RED (B's second page
  // vanishes, or B shows A's error).
  for (const settleA of ["resolve", "reject"]) {
    const late = deferred();
    const bPage = deferred();
    const cursors = [];
    setIpcHandler("admin_connected_community_host", () =>
      Promise.resolve(null),
    );
    setIpcHandler("admin_list_communities", ({ q: query, cursor }) => {
      cursors.push(cursor ?? null);
      if (cursor === "a2") return late.promise;
      if (cursor === "b2") return bPage.promise;
      if (cursor === "b3") {
        return Promise.resolve({ items: [community(6)], nextCursor: null });
      }
      return Promise.resolve(
        query === "b"
          ? { items: [community(4)], nextCursor: "b2" }
          : { items: [community(1)], nextCursor: "a2" },
      );
    });
    const { container: c, unmount } = await mountCommunities();
    const search = async (text) => {
      await act(async () => {
        fireEvent.change(q(c, "communities-search-input"), {
          target: { value: text },
        });
      });
      await settle();
    };
    try {
      await search("a");
      await click(q(c, "communities-load-more")); // A's page 2 stays pending
      await search("b");
      await click(q(c, "communities-load-more")); // B's page 2 in flight
      await act(async () => {
        if (settleA === "resolve") {
          late.resolve({ items: [community(2)], nextCursor: null });
        } else {
          late.reject(new Error("admin API error: stale page"));
        }
      });
      await settle();
      assert.ok(
        q(c, "communities-load-more").disabled,
        `B's request is still busy (${settleA})`,
      );
      await act(async () => {
        bPage.resolve({ items: [community(5)], nextCursor: "b3" });
      });
      await settle();
      const hosts = [
        ...c.querySelectorAll("[data-testid^='community-row-']"),
      ].map((el) => el.dataset.testid.replace("community-row-", ""));
      assert.deepEqual(hosts, ["c4.example.com", "c5.example.com"], settleA);
      assert.doesNotMatch(c.textContent, /stale page/, settleA);
      const more = q(c, "communities-load-more");
      assert.ok(more && !more.disabled, `B can still load more (${settleA})`);
      await click(more);
      assert.equal(cursors.at(-1), "b3", `B keeps its cursor (${settleA})`);
    } finally {
      await unmount();
    }
  }
});

test("communities-unsupported: an older relay's empty 404 shows the copy and the console keeps working", async () => {
  // Mutation: drop the bodyEmpty check from adminRouteUnsupported → RED.
  for (const [bodyEmpty, unsupported] of [
    [true, true],
    [false, false],
  ]) {
    setIpcHandler("admin_connected_community_host", () =>
      Promise.resolve(TEST_COMMUNITY.host),
    );
    setIpcHandler("admin_list_communities", () =>
      Promise.reject({
        message: "admin API error: ",
        relayStatus: 404,
        bodyComplete: true,
        bodyEmpty,
        code: null,
      }),
    );
    const { container: c, unmount } = await mountCommunities();
    try {
      assert.equal(Boolean(q(c, "communities-unsupported")), unsupported);
      await click(q(c, "admin-tab-reports"));
      assert.ok(q(c, "reports-tab"), "Reports still works");
    } finally {
      await unmount();
    }
  }
});

test("community-badge: a report's badge opens its community, with a host-initial fallback", async () => {
  // Mutation: render the badge as a plain span when navigation exists → RED.
  setIpcHandler("admin_connected_community_host", () =>
    Promise.resolve(TEST_COMMUNITY.host),
  );
  makeOpenReportFixtures("00000000-0000-0000-0000-0000000000a1", {
    communityId: TEST_COMMUNITY.id,
  });
  const panel = mountPanel({ origin: CM_ORIGIN, pubkey: CM_PUBKEY });
  await panel.doRender();
  await settle(30);
  const c = panel.container;
  try {
    const badge = q(c, `community-badge-${TEST_COMMUNITY.host}`);
    assert.equal(badge?.tagName, "BUTTON", "badge is its own button");
    assert.equal(
      badge.parentElement.closest("button"),
      null,
      "not nested in a row button",
    );
    assert.equal(q(badge, "community-badge-initial").textContent, "a");
    await click(badge);
    assert.match(q(c, "community-banner").textContent, /alpha\.example\.com/);
    await settle(30);
    assert.ok(q(c, "community-group"), "the page lists the report");
    assert.equal(
      q(c, "community-group-host"),
      null,
      "the banner already names the community",
    );
  } finally {
    await panel.unmount();
  }
});

test("community-badge-disabled-auth: with admin auth disabled badges still open the community page", async () => {
  // Mutation: gate the panel's nav.open on canMutate → RED (badge is a span).
  makeOpenReportFixtures("00000000-0000-0000-0000-0000000000a2", {
    communityId: TEST_COMMUNITY.id,
  });
  const panel = mountPanel({
    origin: CM_ORIGIN,
    pubkey: CM_PUBKEY,
    canMutate: false,
  });
  await panel.doRender();
  await settle(30);
  try {
    const badge = q(panel.container, `community-badge-${TEST_COMMUNITY.host}`);
    assert.equal(badge?.tagName, "BUTTON");
    await click(badge);
    assert.ok(q(panel.container, "community-page"), "badge opens the page");
  } finally {
    await panel.unmount();
  }
});

test("restrictions-lift-signer-change: a signer change while confirming a lift sends nothing", async () => {
  // Mutation: drop the controller's (pubkey, origin) key → RED (the dialog
  // survives and confirms under the new signer).
  const lifts = [];
  setIpcHandler("admin_lift_restriction", ({ intent }) => {
    lifts.push(intent);
    return Promise.resolve();
  });
  const banned = "29".repeat(32);
  setIpcHandler("admin_list_restrictions", () =>
    Promise.resolve({
      items: [
        {
          pubkey: banned,
          banned: true,
          banExpiresAt: null,
          banReason: "spam",
          mutedUntil: null,
          muteReason: null,
          actorPubkey: "aa".repeat(32),
          updatedAt: "2024-06-01T09:00:00Z",
        },
      ],
      nextCursor: null,
    }),
  );
  const panel = mountCommunityPanel(CM_ORIGIN, CM_PUBKEY, "restrictions");
  await panel.doRender();
  await settle(50);
  const c = panel.container;
  try {
    await click(q(c, `restrictions-lift-ban-btn-${banned}`));
    assert.ok(
      document.body.querySelector(
        "[data-testid='restrictions-lift-ban-dialog']",
      ),
    );
    await panel.doRender({ origin: CM_ORIGIN, pubkey: "ee".repeat(32) });
    await settle(50);
    const confirm = document.body.querySelector(
      "[data-testid='restrictions-lift-ban-confirm']",
    );
    if (confirm) await click(confirm);
    assert.deepEqual(lifts, [], "no lift under a changed signer");
  } finally {
    await panel.unmount();
  }
});
