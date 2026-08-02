import assert from "node:assert/strict";
import test from "node:test";

import {
  closestPane,
  DOCUMENT_POSITION_FOLLOWING,
  isElementWithinPane,
  isExcludedChrome,
  isWithinInteractiveText,
  isWithinOverlay,
  resolveClampBoundary,
  shouldClampSelectionToPane,
} from "./conversationSelectionScope.ts";

// ---------------------------------------------------------------------------
// Structural DOM doubles (no jsdom). Only the subset the pure fns touch:
// `closest` / `contains` / `compareDocumentPosition`.
// ---------------------------------------------------------------------------

function stub({ closestMap = {}, contains = () => false } = {}) {
  return {
    closest: (selector) => closestMap[selector] ?? null,
    contains,
    compareDocumentPosition: () => 0,
  };
}

// --- closestPane -----------------------------------------------------------

test("closestPane: element inside a pane returns the pane", () => {
  const pane = stub();
  const el = stub({ closestMap: { "[data-selection-pane]": pane } });
  assert.equal(closestPane(el), pane);
});

test("closestPane: element outside any pane returns null", () => {
  assert.equal(closestPane(stub()), null);
});

test("closestPane: null-safe", () => {
  assert.equal(closestPane(null), null);
  assert.equal(closestPane(undefined), null);
});

// --- isExcludedChrome ------------------------------------------------------

test("isExcludedChrome: hover chrome marked excluded → true", () => {
  const marker = stub();
  const el = stub({ closestMap: { "[data-selection-exclude]": marker } });
  assert.equal(isExcludedChrome(el), true);
});

test("isExcludedChrome: message text → false", () => {
  assert.equal(isExcludedChrome(stub()), false);
});

test("isExcludedChrome: null-safe", () => {
  assert.equal(isExcludedChrome(null), false);
});

// --- isWithinInteractiveText ------------------------------------------------

test("isWithinInteractiveText: textarea → true (composer/IME guard)", () => {
  const el = stub({ closestMap: { "input,textarea,[contenteditable],[contenteditable='true']": {} } });
  assert.equal(isWithinInteractiveText(el), true);
});

test("isWithinInteractiveText: plain message div → false", () => {
  assert.equal(isWithinInteractiveText(stub()), false);
});

// --- isWithinOverlay --------------------------------------------------------

test("isWithinOverlay: Radix dialog content → true", () => {
  const el = stub({
    closestMap: {
      "[data-radix-dialog],[role='dialog'],[data-radix-popper-content-wrapper]": {},
    },
  });
  assert.equal(isWithinOverlay(el), true);
});

test("isWithinOverlay: in-pane content → false", () => {
  assert.equal(isWithinOverlay(stub()), false);
});

// --- isElementWithinPane ----------------------------------------------------

test("isElementWithinPane: same element → true", () => {
  const pane = stub();
  assert.equal(isElementWithinPane(pane, pane), true);
});

test("isElementWithinPane: descendant → true via contains", () => {
  const child = stub();
  const pane = stub({ contains: (o) => o === child });
  assert.equal(isElementWithinPane(child, pane), true);
});

test("isElementWithinPane: sibling pane content → false", () => {
  const other = stub();
  const channelPane = stub({ contains: () => false });
  assert.equal(isElementWithinPane(other, channelPane), false);
});

// --- shouldClampSelectionToPane --------------------------------------------

test("shouldClamp: no anchor pane → null (drag began outside conversation)", () => {
  assert.equal(shouldClampSelectionToPane(null, stub()), null);
});

test("shouldClamp: focus still in pane → null", () => {
  const pane = stub();
  assert.equal(shouldClampSelectionToPane(pane, pane), null);
});

test("shouldClamp: focus escaped into sibling pane → clamp to anchor", () => {
  const channelPane = stub({ contains: () => false });
  const threadPane = stub();
  const threadContent = stub({
    closestMap: { "[data-selection-pane]": threadPane },
  });
  assert.equal(
    shouldClampSelectionToPane(channelPane, threadContent),
    channelPane,
    "channel drag escaping into thread must clamp back to channel",
  );
});

test("shouldClamp: focus in textarea/composer → null (IME never disturbed)", () => {
  const pane = stub({ contains: () => false });
  const composer = stub({
    closestMap: { "input,textarea,[contenteditable],[contenteditable='true']": {} },
  });
  assert.equal(shouldClampSelectionToPane(pane, composer), null);
});

test("shouldClamp: focus in dialog overlay → null (dialog owns selection)", () => {
  const pane = stub({ contains: () => false });
  const dialog = stub({
    closestMap: {
      "[data-radix-dialog],[role='dialog'],[data-radix-popper-content-wrapper]": {},
    },
  });
  assert.equal(shouldClampSelectionToPane(pane, dialog), null);
});

// --- resolveClampBoundary --------------------------------------------------

test("resolveClampBoundary: missing args → after", () => {
  assert.equal(resolveClampBoundary(null, null), "after");
  assert.equal(resolveClampBoundary(stub(), null), "after");
});

test("resolveClampBoundary: focus before pane → before", () => {
  const pane = stub();
  const before = { compareDocumentPosition: () => DOCUMENT_POSITION_FOLLOWING };
  assert.equal(resolveClampBoundary(pane, before), "before");
});

test("resolveClampBoundary: focus after pane → after", () => {
  const pane = stub();
  const after = { compareDocumentPosition: () => 0 };
  assert.equal(resolveClampBoundary(pane, after), "after");
});

// --- reporter matrix --------------------------------------------------------

test("reporter matrix: channel selection clips at channel edge (block/buzz#4077)", () => {
  const channelPane = stub({ contains: () => false });
  const threadPane = stub();
  const threadContent = stub({
    closestMap: { "[data-selection-pane]": threadPane },
  });
  const clampTarget = shouldClampSelectionToPane(channelPane, threadContent);
  assert.ok(clampTarget, "selection escaping channel must clamp");
  const boundary = resolveClampBoundary(clampTarget, threadContent);
  assert.ok(["before", "after"].includes(boundary));
});

test("reporter matrix: message text remains selectable and un-excluded", () => {
  const pane = stub();
  const text = stub({ closestMap: { "[data-selection-pane]": pane } });
  pane.contains = (o) => o === text;
  assert.equal(shouldClampSelectionToPane(pane, text), null);
  assert.equal(isExcludedChrome(text), false);
});

test("reporter matrix: hover chrome is excluded from selection", () => {
  const marker = stub();
  const chrome = stub({ closestMap: { "[data-selection-exclude]": marker } });
  assert.equal(isExcludedChrome(chrome), true);
});