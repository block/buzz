import assert from "node:assert/strict";
import test from "node:test";
import { revalidateChannelWindow } from "./revalidateChannelWindow.ts";
import {
  appendOlderChannelWindow,
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
} from "./channelWindowStore.ts";
const event = (n) => ({ id: String(n), created_at: n });
const cursor = (n) => ({ eventId: String(n), createdAt: n });
const page = (start, ids, more = true) => ({
  startCursor: start,
  rows: ids.map((n) => ({ event: event(n), thread: null })),
  aux: [],
  hasMore: more,
  nextCursor: more ? cursor(ids.at(-1)) : null,
});
const retained = () =>
  appendOlderChannelWindow(
    replaceNewestChannelWindow(emptyChannelWindowStore(), page(null, [10, 9])),
    page(cursor(9), [8, 7]),
  );

test("unchanged head verifies one fresh join page before reusing deeper history", async () => {
  const current = retained();
  const head = page(null, [11, 10, 9]);
  const pages = await revalidateChannelWindow({
    publish: (pages) => pages,
    head,
    retained: current,
    readCurrent: () => current,
    fetchPage: async (start) => {
      assert.deepEqual(start, cursor(9));
      return current.pages[1];
    },
    signal: new AbortController().signal,
  });
  assert.deepEqual(pages, [head, current.pages[1]]);
});

test("moved head stages a replacement chain before publishing deep history", async () => {
  const current = retained();
  const head = page(null, [12, 11]);
  const fetched = [];
  const pages = await revalidateChannelWindow({
    publish: (pages) => pages,
    head,
    retained: current,
    readCurrent: () => current,
    fetchPage: async (start) => {
      fetched.push(start);
      assert.equal(
        current.pages.length,
        2,
        "published view remains unchanged during requests",
      );
      return start.createdAt === 11 ? page(start, [10, 9]) : current.pages[1];
    },
    signal: new AbortController().signal,
  });
  assert.deepEqual(fetched, [cursor(11), cursor(9)]);
  assert.deepEqual(pages, [head, page(cursor(11), [10, 9]), current.pages[1]]);
});

test("changed boundaries follow only fresh cursors until the old range is covered", async () => {
  const current = retained();
  const head = page(null, [13, 12]);
  const pages = await revalidateChannelWindow({
    publish: (pages) => pages,
    head,
    retained: current,
    readCurrent: () => current,
    fetchPage: async (start) =>
      page(start, [start.createdAt - 1, start.createdAt - 2]),
    signal: new AbortController().signal,
  });
  assert.deepEqual(
    pages.map((p) => p.rows.map((r) => r.event.id)),
    [
      ["13", "12"],
      ["11", "10"],
      ["9", "8"],
      ["7", "6"],
    ],
  );
});

test("failure, budget and cancellation preserve the old authoritative store", async () => {
  const current = retained();
  const original = structuredClone(current);
  const input = {
    head: page(null, [100, 99]),
    retained: current,
    readCurrent: () => current,
    signal: new AbortController().signal,
  };
  await assert.rejects(
    revalidateChannelWindow({
      publish: (pages) => pages,
      ...input,
      fetchPage: async () => {
        throw Error("offline");
      },
    }),
    /offline/,
  );
  let requests = 0;
  await assert.rejects(
    revalidateChannelWindow({
      publish: (pages) => pages,
      ...input,
      fetchPage: async (start) => {
        requests++;
        return page(start, [start.createdAt - 1]);
      },
    }),
    /budget/,
  );
  assert.equal(requests, 5);
  const abort = new AbortController();
  abort.abort();
  await assert.rejects(
    revalidateChannelWindow({
      publish: (pages) => pages,
      ...input,
      signal: abort.signal,
      fetchPage: async () => {
        throw Error("must not fetch");
      },
    }),
    { name: "AbortError" },
  );
  assert.deepEqual(current, original);
});

test("concurrent paging during staged fetch extends the required reading depth before synchronous publication", async () => {
  let current = retained();
  let published;
  const pages = await revalidateChannelWindow({
    head: page(null, [12, 11]),
    retained: current,
    readCurrent: () => current,
    signal: new AbortController().signal,
    fetchPage: async (start) => {
      if (start.createdAt === 11) {
        current = appendOlderChannelWindow(current, page(cursor(7), [6, 5]));
        return page(start, [10, 9]);
      }
      return current.pages[1];
    },
    publish: (pages) => {
      published = pages;
      return pages;
    },
  });
  assert.equal(pages, published);
  assert.deepEqual(
    pages.map((p) => p.rows.map((r) => r.event.id)),
    [
      ["12", "11"],
      ["10", "9"],
      ["8", "7"],
      ["6", "5"],
    ],
  );
});

test("even a reader in the first page retains their old boundary on head refresh", async () => {
  const current = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    page(null, [10, 9]),
  );
  const pages = await revalidateChannelWindow({
    head: page(null, [12, 11]),
    retained: current,
    readCurrent: () => current,
    signal: new AbortController().signal,
    fetchPage: async (start) => page(start, [10, 9]),
    publish: (pages) => pages,
  });
  assert.deepEqual(
    pages.map((p) => p.rows.map((r) => r.event.id)),
    [
      ["12", "11"],
      ["10", "9"],
    ],
  );
});

// Relay-shaped source with composite ordering and candidate-based cursors.
// Compare the published sequence against server truth, not just old-row retention.
function model(rows, skipped = new Set()) {
  const ordered = [...rows].sort(
    (a, b) => b.created_at - a.created_at || a.id.localeCompare(b.id),
  );
  return (start, limit = 50) => {
    const eligible = ordered.filter(
      (row) =>
        !start ||
        row.created_at < start.createdAt ||
        (row.created_at === start.createdAt && row.id > start.eventId),
    );
    const candidates = eligible.slice(0, limit);
    const last = candidates.at(-1);
    const more = eligible.length > candidates.length;
    return {
      startCursor: start,
      rows: candidates
        .filter((row) => !skipped.has(row.id))
        .map((event) => ({ event, thread: null })),
      aux: [],
      hasMore: more,
      nextCursor: more
        ? { createdAt: last.created_at, eventId: last.id }
        : null,
    };
  };
}
function deepStore(fetch, depth = 20) {
  let current = replaceNewestChannelWindow(
    emptyChannelWindowStore(),
    fetch(null),
  );
  for (let n = 1; n < depth; n++)
    current = appendOlderChannelWindow(
      current,
      fetch(current.pages.at(-1).nextCursor),
    );
  return current;
}
const sourceRows = () =>
  Array.from({ length: 2000 }, (_, index) => ({
    id: String(5000 - index).padStart(64, "0"),
    created_at: 5000 - index,
  }));
async function refreshModel(current, fetch) {
  const requests = [];
  const pages = await revalidateChannelWindow({
    head: fetch(null),
    retained: current,
    readCurrent: () => current,
    fetchPage: async (cursor, limit) => {
      requests.push(limit);
      return fetch(cursor, limit);
    },
    signal: new AbortController().signal,
    publish: (pages) => pages,
  });
  return { pages, requests };
}
function assertContiguous(pages, fetch) {
  const actual = pages.flatMap((page) => page.rows.map((row) => row.event.id));
  const expected = fetch(null, Infinity)
    .rows.slice(0, actual.length)
    .map((row) => row.event.id);
  assert.deepEqual(actual, expected);
}
for (const added of [1, 7, 50, 137, 201, 250, 400]) {
  test(`${added} head arrivals join without re-fetching twenty retained pages`, async () => {
    const rows = sourceRows();
    const current = deepStore(model(rows));
    const fetch = model([
      ...rows,
      ...Array.from({ length: added }, (_, n) => ({
        id: `new-${n}`,
        created_at: 6000 + n,
      })),
    ]);
    const { pages, requests } = await refreshModel(current, fetch);
    assertContiguous(pages, fetch);
    assert.ok(
      pages
        .flatMap((p) => p.rows)
        .some((r) => r.event.id === current.pages.at(-1).rows.at(-1).event.id),
    );
    assert.ok(
      requests.length <= Math.ceil(added / 50) + 1,
      JSON.stringify(requests),
    );
    if (added === 1) assert.deepEqual(requests, [1, 50]);
  });
}
test("fresh join verification includes a new dense-second row immediately after the old boundary", async () => {
  const rows = sourceRows();
  const current = deepStore(model(rows));
  const boundary = current.pages[0].rows.at(-1).event;
  const twin = { id: "f".repeat(64), created_at: boundary.created_at };
  const fetch = model([...rows, { id: "new", created_at: 6000 }, twin]);
  const { pages } = await refreshModel(current, fetch);
  assertContiguous(pages, fetch);
  assert.ok(pages.flatMap((p) => p.rows).some((r) => r.event.id === twin.id));
});
for (const skip of [false, true]) {
  test(`${skip ? "skipped scan candidate" : "deleted boundary"} cannot invent a join or lose retained history`, async () => {
    const rows = sourceRows();
    const boundary = rows[49].id;
    const skipped = new Set(skip ? [boundary] : []);
    const current = deepStore(model(rows, skipped));
    const fetch = model(
      [
        ...rows.filter((row) => skip || row.id !== boundary),
        { id: "new", created_at: 6000 },
      ],
      skipped,
    );
    const { pages, requests } = await refreshModel(current, fetch);
    assertContiguous(pages, fetch);
    assert.ok(requests.length < 5, JSON.stringify(requests));
  });
}
