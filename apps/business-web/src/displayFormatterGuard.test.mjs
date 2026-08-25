import assert from "node:assert/strict";
import test from "node:test";
import { findUnformattedBusinessDisplays } from "../scripts/check-display-formatters.mjs";

test("detects raw business decimals rendered in JSX", () => {
  const source = `
    function View({ row }) {
      return <><strong>{row.grossAmount}</strong><span>{row.quantity ?? "0"}</span><i>{row.inventoryValue}</i></>;
    }
  `;

  assert.deepEqual(
    findUnformattedBusinessDisplays(source).map(({ field }) => field),
    ["grossAmount", "quantity", "inventoryValue"],
  );
});

test("allows shared formatters and non-rendering JSX attributes", () => {
  const source = `
    function View({ row }) {
      return <input max={row.availableQuantity} value={formatAmount(row.grossAmount)} />;
    }
  `;

  assert.deepEqual(findUnformattedBusinessDisplays(source), []);
});
