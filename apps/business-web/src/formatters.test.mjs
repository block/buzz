import assert from "node:assert/strict";
import test from "node:test";
import {
  fixedDecimal,
  formatAmount,
  formatMoney,
  formatQuantity,
  formatSignedQuantity,
} from "./formatters.ts";

test("amounts and quantities always display two decimal places", () => {
  assert.equal(formatAmount("1053.500000"), "1,053.50");
  assert.equal(formatQuantity(1), "1.00");
  assert.equal(formatMoney("CNY", "600.000000"), "CNY 600.00");
  assert.equal(formatSignedQuantity("12.5"), "+12.50");
  assert.equal(formatSignedQuantity("-2"), "-2.00");
});

test("input normalization stays ungrouped", () => {
  assert.equal(fixedDecimal("1234.5"), "1234.50");
  assert.equal(formatAmount(null), "—");
});
