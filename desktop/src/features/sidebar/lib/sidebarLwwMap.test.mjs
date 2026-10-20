import assert from "node:assert/strict";
import test from "node:test";

import {
  canonical,
  compareRegs,
  LEGACY_DEV,
  mergeTrees,
  setRegs,
  validTree,
} from "./sidebarLwwMap.ts";

const storage = new Map();
globalThis.window ??= {};
globalThis.window.localStorage = {
  getItem: (k) => storage.get(k) ?? null,
  setItem: (k, v) => storage.set(k, v),
  removeItem: (k) => storage.delete(k),
};

const A = "a".repeat(16);
const B = "b".repeat(16);

test("compareRegs: version, then device, then tombstone, then value bytes", () => {
  assert.ok(compareRegs([2, A, "x"], [1, B, "y"]) > 0);
  assert.ok(compareRegs([1, B, "x"], [1, A, "y"]) > 0);
  assert.ok(compareRegs([1, A, null], [1, A, "x"]) > 0);
  assert.ok(compareRegs([1, A, false], [1, A, true]) > 0);
  assert.ok(compareRegs([1, A, "b"], [1, A, "a"]) > 0);
  assert.equal(compareRegs([1, A, "a"], [1, A, "a"]), 0);
});

test("mergeTrees: commutative, associative, idempotent", () => {
  const x = { s: { k: { name: [1, A, "x"] } }, a: { c1: [5, A, "k"] } };
  const y = { s: { k: { name: [2, B, "y"], live: [1, B, true] } } };
  const z = { a: { c1: [5, B, null], c2: [3, A, "k"] } };
  const xy = mergeTrees(x, y);
  assert.equal(canonical(xy), canonical(mergeTrees(y, x)));
  assert.equal(
    canonical(mergeTrees(mergeTrees(x, y), z)),
    canonical(mergeTrees(x, mergeTrees(y, z))),
  );
  assert.equal(mergeTrees(xy, xy), xy);
  assert.deepEqual(xy.s.k.name, [2, B, "y"]);
});

test("mergeTrees: returns the same reference when nothing changes", () => {
  const a = { g: { dms: [9, A, "recent"] } };
  assert.equal(mergeTrees(a, { g: { dms: [1, B, "alpha"] } }), a);
});

test("mergeTrees onlyMissing: fills absent leaves and never overrides", () => {
  const local = { s: { k: { name: [1, A, "mine"] } } };
  const legacy = {
    s: {
      k: { name: [9e12, LEGACY_DEV, "old"] },
      n: { name: [9e12, LEGACY_DEV, "new"] },
    },
  };
  const out = mergeTrees(local, legacy, true);
  assert.equal(out.s.k.name[2], "mine");
  assert.equal(out.s.n.name[2], "new");
});

test("setRegs: identical edit mints no version and returns the same tree", () => {
  const t = setRegs({}, [[["g", "dms"], "recent"]]);
  assert.equal(setRegs(t, [[["g", "dms"], "recent"]]), t);
});

test("setRegs: new version exceeds every version seen and the install's last", () => {
  const future = Date.now() + 1e9;
  const t = setRegs({ g: { dms: [future, B, "alpha"] } }, [
    [["g", "dms"], "recent"],
  ]);
  assert.equal(t.g.dms[0], future + 1);
  const t2 = setRegs({}, [[["g", "x"], "alpha"]]);
  assert.ok(t2.g.x[0] > future, "lastIssued persists across documents");
  assert.equal(t2.g.x[1], t.g.dms[1], "device id persists per install");
  assert.match(t2.g.x[1], /^[0-9a-f]{16}$/);
});

test("setRegs all: writes every leaf, including unchanged ones", () => {
  const t = { s: { a: { order: [1, A, 0] }, b: { order: [1, A, 1] } } };
  const out = setRegs(
    t,
    [
      [["s", "a", "order"], 0],
      [["s", "b", "order"], 0],
    ],
    true,
  );
  assert.ok(out.s.a.order[0] > 1);
  assert.equal(out.s.a.order[0], out.s.b.order[0]);
});

test("validTree: skips invalid records, keeps valid siblings", () => {
  const shape = {
    s: {
      "*": { name: (v) => typeof v === "string", order: Number.isSafeInteger },
    },
  };
  const out = validTree(
    {
      s: {
        ok: { name: [1, A, "n"], order: [1, A, 2] },
        badName: { name: [1, A, 7], order: [1, A, 0] },
        badDev: { name: [1, "zz", "n"] },
        badV: { name: [1.5, A, "n"] },
        __proto__: { name: [1, A, "p"] },
      },
      extra: { x: [1, A, 1] },
    },
    shape,
  );
  assert.deepEqual(out, {
    s: {
      ok: { name: [1, A, "n"], order: [1, A, 2] },
      badName: { order: [1, A, 0] },
      badDev: {},
      badV: {},
    },
  });
});

test("canonical: key order independent and stable", () => {
  assert.equal(
    canonical({ b: 1, a: { d: [1], c: null } }),
    canonical({ a: { c: null, d: [1] }, b: 1 }),
  );
  assert.equal(canonical({ a: undefined, b: 1 }), '{"b":1}');
});

test("inherited names are ordinary keys in either arrival order", () => {
  const A = "a".repeat(16);
  const one = { g: { constructor: [2, A, "recent"] } };
  const two = { g: { toString: [3, A, "alpha"] } };
  const ab = mergeTrees(mergeTrees({}, one), two);
  const ba = mergeTrees(mergeTrees({}, two), one);
  assert.equal(canonical(ab), canonical(ba));
  assert.deepEqual(Object.keys(ab.g).sort(), ["constructor", "toString"]);
  const set = setRegs({ g: {} }, [[["g", "constructor"], "alpha"]]);
  assert.equal(set.g.constructor[2], "alpha");
  const valid = validTree(JSON.parse(canonical(ab)), {
    g: { "*": () => true },
  });
  assert.equal(canonical(valid), canonical(ab));
});
