import assert from "node:assert/strict";
import test from "node:test";

import {
  filterGoToDestinations,
  resolveDigitAccelerator,
  resolveMnemonicAccelerator,
  selectEnabledDestinations,
} from "./goToAccelerators.ts";

const inbox = { label: "Inbox", mnemonic: "I", keywords: ["home", "feed"] };
const agents = { label: "Agents", mnemonic: "A", keywords: ["bots"] };
const projects = { label: "Projects", mnemonic: "P", feature: "projects" };
const items = [inbox, agents, projects];

test("selectEnabledDestinations keeps always-on areas and enabled gated areas", () => {
  const enabled = selectEnabledDestinations(items, () => true);
  assert.deepEqual(enabled, [inbox, agents, projects]);
});

test("selectEnabledDestinations hides gated-off areas but keeps always-on ones", () => {
  const enabled = selectEnabledDestinations(
    items,
    (feature) => feature !== "projects",
  );
  assert.deepEqual(enabled, [inbox, agents]);
});

test("filterGoToDestinations returns everything for an empty query", () => {
  assert.deepEqual(filterGoToDestinations(items, "   "), items);
});

test("filterGoToDestinations matches label and keywords, case-insensitively", () => {
  assert.deepEqual(filterGoToDestinations(items, "AGE"), [agents]);
  assert.deepEqual(filterGoToDestinations(items, "feed"), [inbox]);
  assert.deepEqual(filterGoToDestinations(items, "zzz"), []);
});

test("resolveDigitAccelerator jumps by 1-based visible position", () => {
  const visible = [agents, inbox];
  assert.equal(resolveDigitAccelerator(visible, "1"), agents);
  assert.equal(resolveDigitAccelerator(visible, "2"), inbox);
});

test("resolveDigitAccelerator rejects non-digits, zero, and out-of-range", () => {
  assert.equal(resolveDigitAccelerator(items, "0"), null);
  assert.equal(resolveDigitAccelerator(items, "9"), null);
  assert.equal(resolveDigitAccelerator(items, "a"), null);
  assert.equal(resolveDigitAccelerator(items, ""), null);
});

test("resolveMnemonicAccelerator is a global, case-insensitive letter match", () => {
  assert.equal(resolveMnemonicAccelerator(items, "i"), inbox);
  assert.equal(resolveMnemonicAccelerator(items, "A"), agents);
  // Global: matches even when the item would be filtered out of a visible view.
  assert.equal(resolveMnemonicAccelerator(items, "p"), projects);
  assert.equal(resolveMnemonicAccelerator(items, "z"), null);
  assert.equal(resolveMnemonicAccelerator(items, "in"), null);
});
