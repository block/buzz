import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  React,
  act,
  createRoot,
  deferred,
  fireEvent,
} from "../../admin-console/adminConsolePanelTestHelpers.jsdom.mjs";
import { EmojiCatalogSearch } from "./EmojiCatalogSearch.tsx";

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

const emoji = (shortcode) => ({ shortcode, url: `https://x/${shortcode}.png` });

function mount(searcher) {
  const selected = [];
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() => {
    root.render(
      React.createElement(EmojiCatalogSearch, {
        searcher,
        onSelect: (e) => selected.push(e.shortcode),
      }),
    );
  });
  return selected;
}

const input = () =>
  container.querySelector("[data-testid='emoji-catalog-search']");
const options = () => [...container.querySelectorAll("[role='option']")];
const status = () => container.querySelector("[role='status']").textContent;
const type = (value) =>
  act(() => {
    fireEvent.change(input(), { target: { value } });
  });
const key = (k, init = {}) =>
  act(() => {
    fireEvent.keyDown(input(), { key: k, ...init });
  });
const flush = () => act(async () => {});

test("typing searches, the list is one label owner per option, and the keyboard drives it", async () => {
  const requests = [];
  const searcher = {
    request: async (q) => {
      requests.push(q);
      if (q.trim().length < 2) return { status: "short" };
      return {
        status: "ok",
        prefix: q,
        results: [emoji("party_parrot"), emoji("parrot"), emoji("pars")],
      };
    },
    reset() {},
  };
  const selected = mount(searcher);

  type("p");
  await flush();
  assert.equal(options().length, 0);
  assert.equal(input().getAttribute("aria-expanded"), "false");

  type("par");
  await flush();
  assert.equal(options().length, 3);
  assert.equal(status(), "3 results");
  assert.equal(input().getAttribute("role"), "combobox");
  assert.equal(input().getAttribute("aria-expanded"), "true");
  for (const option of options()) {
    assert.ok(option.getAttribute("aria-label")?.startsWith(":"));
    assert.equal(option.querySelector("img").getAttribute("alt"), "");
    assert.equal(
      option.querySelector("[aria-hidden='true']").getAttribute("aria-hidden"),
      "true",
    );
  }
  const activeId = () => input().getAttribute("aria-activedescendant");
  assert.equal(activeId(), options()[0].id);

  key("ArrowDown");
  assert.equal(activeId(), options()[1].id);
  key("ArrowUp");
  key("ArrowUp");
  assert.equal(activeId(), options()[2].id);

  key(" ", { shiftKey: true });
  key("Tab");
  key("Enter", { shiftKey: true });
  assert.deepEqual(selected, []);

  key("Enter");
  assert.deepEqual(selected, ["pars"]);

  key("Escape");
  assert.equal(input().value, "");
  assert.equal(options().length, 0);
});

test("the hint appears only at the result cap, and an empty result is stated", async () => {
  const many = Array.from({ length: 50 }, (_, i) => emoji(`e${i}`));
  let reply = many;
  const searcher = {
    request: async () => ({ status: "ok", prefix: "e", results: reply }),
    reset() {},
  };
  mount(searcher);
  type("ee");
  await flush();
  assert.match(status(), /Keep typing to narrow/);

  reply = [];
  type("eee");
  await flush();
  assert.equal(status(), "No community emoji match.");
});

test("a late reply for an older query never replaces a newer one, and a failure offers retry", async () => {
  const slow = deferred();
  const replies = new Map([
    ["aa", slow.promise],
    [
      "bb",
      Promise.resolve({
        status: "ok",
        prefix: "bb",
        results: [emoji("bb_new")],
      }),
    ],
  ]);
  let failBB = false;
  const searcher = {
    request: (q) => {
      if (q === "bb" && failBB) {
        return Promise.resolve({
          status: "error",
          prefix: q,
          error: new Error("x"),
        });
      }
      return replies.get(q) ?? Promise.resolve({ status: "short" });
    },
    reset() {},
  };
  mount(searcher);
  type("aa");
  type("bb");
  await flush();
  slow.resolve({ status: "ok", prefix: "aa", results: [emoji("aa_old")] });
  await flush();
  assert.deepEqual(
    options().map((o) => o.getAttribute("aria-label")),
    [":bb_new:"],
  );

  failBB = true;
  type("b");
  type("bb");
  await flush();
  assert.match(status(), /search failed/);
  const retry = container.querySelector("[role='status'] button");
  assert.ok(retry);
  failBB = false;
  act(() => {
    fireEvent.click(retry);
  });
  await flush();
  assert.equal(options().length, 1);
});
