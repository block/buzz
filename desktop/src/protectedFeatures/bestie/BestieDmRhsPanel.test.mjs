import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieCategoryTitle,
  bestieIdleAuxiliaryKind,
  presentBestieContextCount,
} from "./bestieDmRhsHelpers.ts";

test("bestieCategoryTitle labels reminder and todo categories", () => {
  assert.equal(bestieCategoryTitle("reminder"), "Reminders");
  assert.equal(bestieCategoryTitle("todo"), "To-dos");
});

test("presentBestieContextCount omits empty like Projects overview", () => {
  assert.equal(presentBestieContextCount(undefined), undefined);
  assert.equal(presentBestieContextCount(0), undefined);
  assert.equal(presentBestieContextCount(3), 3);
});

test("bestieIdleAuxiliaryKind opens slide only for a drilled category", () => {
  assert.equal(bestieIdleAuxiliaryKind(null), null);
  assert.equal(bestieIdleAuxiliaryKind("reminder"), "reminder");
  assert.equal(bestieIdleAuxiliaryKind("todo"), "todo");
});
