import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

after(() => dom.window.close());

const RELAY = "wss://relay.example";

/** Mounts the hook against a silent relay; returns `[result, teardown]`. */
async function mountSections(pubkey) {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { relayClient } = await import("@/shared/api/relayClient");
  const { useChannelSections } = await import("./useChannelSections.ts");

  const originals = {
    fetchEvents: relayClient.fetchEvents,
    subscribeLive: relayClient.subscribeLive,
    subscribeToReconnects: relayClient.subscribeToReconnects,
  };
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async () => async () => {};
  relayClient.subscribeToReconnects = () => () => {};

  const { result, unmount } = renderHook(() =>
    useChannelSections(pubkey, RELAY),
  );
  return [
    result,
    act,
    () => {
      unmount();
      cleanup();
      Object.assign(relayClient, originals);
    },
  ];
}

const idOf = (result, name) =>
  result.current.sections.find((section) => section.name === name)?.id;

test("createSection nests under a top-level section", async () => {
  const [result, act, teardown] = await mountSections("pk-create");
  try {
    let parentId;
    act(() => {
      parentId = result.current.createSection("Work")?.id;
    });
    act(() => {
      result.current.createSection("Alpha", undefined, parentId);
    });

    assert.deepEqual(
      result.current.sections.map((s) => [s.name, s.parentId]),
      [
        ["Work", undefined],
        ["Alpha", parentId],
      ],
    );
  } finally {
    teardown();
  }
});

test("createSection refuses a third level and creates at the top instead", async () => {
  const [result, act, teardown] = await mountSections("pk-depth-create");
  try {
    let parentId;
    act(() => {
      parentId = result.current.createSection("Work")?.id;
    });
    act(() => {
      result.current.createSection("Alpha", undefined, parentId);
    });
    const childId = idOf(result, "Alpha");
    let created;
    act(() => {
      created = result.current.createSection("Deep", undefined, childId);
    });

    assert.equal(created?.parentId, undefined);
    assert.equal(idOf(result, "Deep") !== undefined, true);
    assert.equal(
      result.current.sections.find((s) => s.name === "Deep")?.parentId,
      undefined,
    );
  } finally {
    teardown();
  }
});

test("setSectionParent refuses self-nesting, a nested target, and a parent with children", async () => {
  const [result, act, teardown] = await mountSections("pk-set-parent");
  try {
    act(() => {
      result.current.createSection("Work");
    });
    const workId = idOf(result, "Work");
    act(() => {
      result.current.createSection("Alpha", undefined, workId);
    });
    act(() => {
      result.current.createSection("Clients");
    });
    const alphaId = idOf(result, "Alpha");
    const clientsId = idOf(result, "Clients");

    act(() => result.current.setSectionParent(clientsId, clientsId));
    assert.equal(idOf(result, "Clients"), clientsId);
    assert.equal(
      result.current.sections.find((s) => s.id === clientsId)?.parentId,
      undefined,
    );

    // Alpha is already nested, so it cannot become a parent.
    act(() => result.current.setSectionParent(clientsId, alphaId));
    assert.equal(
      result.current.sections.find((s) => s.id === clientsId)?.parentId,
      undefined,
    );

    // Work has a child, so it cannot itself be nested.
    act(() => result.current.setSectionParent(workId, clientsId));
    assert.equal(
      result.current.sections.find((s) => s.id === workId)?.parentId,
      undefined,
    );

    // A refused parent must not be written: Alpha moving to the top level
    // would otherwise reparent Clients under it later.
    act(() => result.current.setSectionParent(alphaId, null));
    assert.equal(
      result.current.sections.find((s) => s.id === clientsId)?.parentId,
      undefined,
    );

    // A plain top-level section nests, and lifts back out.
    act(() => result.current.setSectionParent(clientsId, workId));
    assert.equal(
      result.current.sections.find((s) => s.id === clientsId)?.parentId,
      workId,
    );
    act(() => result.current.setSectionParent(clientsId, null));
    assert.equal(
      result.current.sections.find((s) => s.id === clientsId)?.parentId,
      undefined,
    );
  } finally {
    teardown();
  }
});

test("moveSectionDown reorders within one sibling group", async () => {
  const [result, act, teardown] = await mountSections("pk-move");
  try {
    act(() => {
      result.current.createSection("Work");
    });
    const workId = idOf(result, "Work");
    act(() => {
      result.current.createSection("Alpha", undefined, workId);
    });
    act(() => {
      result.current.createSection("Beta", undefined, workId);
    });
    act(() => {
      result.current.createSection("Clients");
    });

    act(() => result.current.moveSectionDown(idOf(result, "Alpha")));

    assert.deepEqual(
      result.current.sections.map((s) => s.name),
      ["Work", "Beta", "Alpha", "Clients"],
    );

    // Reordering children left the top-level order alone.
    act(() => result.current.moveSectionDown(workId));
    assert.deepEqual(
      result.current.sections.map((s) => s.name),
      ["Clients", "Work", "Beta", "Alpha"],
    );
  } finally {
    teardown();
  }
});
