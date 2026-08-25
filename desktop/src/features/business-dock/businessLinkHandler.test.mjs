import assert from "node:assert/strict";
import test from "node:test";

import { handleBuzzLinkClick } from "./businessLinkHandler.ts";

const config = {
  homeUrl: "https://biz.example.com/embed/",
  origin: "https://biz.example.com",
};

function run(url, modifiers = {}) {
  const calls = [];
  let prevented = false;
  const result = handleBuzzLinkClick({
    config,
    url,
    event: {
      ctrlKey: false,
      metaKey: false,
      preventDefault: () => {
        prevented = true;
      },
      ...modifiers,
    },
    onOpenBusinessResource: (resource) => calls.push(["dock", resource.id]),
    onOpenExternal: (externalUrl) => calls.push(["external", externalUrl]),
  });
  return { calls, prevented, result };
}

test("ordinary links preserve Buzz default behavior", () => {
  assert.deepEqual(run("https://example.org"), {
    calls: [],
    prevented: false,
    result: "default",
  });
});

test("business HTTPS and biz links open in Business Dock", () => {
  assert.deepEqual(run("biz://sales-order/SO-1"), {
    calls: [["dock", "SO-1"]],
    prevented: true,
    result: "business",
  });
  assert.deepEqual(run("https://biz.example.com/embed/customers/C-1"), {
    calls: [["dock", "C-1"]],
    prevented: true,
    result: "business",
  });
});

test("command or control click opens the allowlisted HTTPS URL externally", () => {
  const output = run("biz://invoice/INV-1", { metaKey: true });
  assert.equal(output.result, "external");
  assert.equal(output.prevented, true);
  assert.deepEqual(output.calls, [
    ["external", "https://biz.example.com/embed/invoices/INV-1"],
  ]);
});
