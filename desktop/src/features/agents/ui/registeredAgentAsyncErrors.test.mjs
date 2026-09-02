import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const registerDialog = readFileSync(
  new URL("./RegisterExistingAgentDialog.tsx", import.meta.url),
  "utf8",
);
const registeredCard = readFileSync(
  new URL("./RegisteredAgentIdentityCard.tsx", import.meta.url),
  "utf8",
);
const agentsView = readFileSync(
  new URL("./AgentsView.tsx", import.meta.url),
  "utf8",
);

test("registration rejection stays handled and leaves the dialog open", () => {
  assert.match(
    registerDialog,
    /try\s*\{[\s\S]*await onSubmit\([\s\S]*onOpenChange\(false\)[\s\S]*\}\s*catch\s*\{/,
  );
});

test("clipboard rejection is handled inside the registered-reference card", () => {
  assert.match(
    registeredCard,
    /try\s*\{[\s\S]*await navigator\.clipboard\?\.writeText\([\s\S]*\}\s*catch\s*\{/,
  );
});

test("closing registration resets stale mutation errors", () => {
  assert.match(
    agentsView,
    /onOpenChange=\{\(open\) => \{[\s\S]*setIsRegisterExistingOpen\(open\)[\s\S]*if \(!open\) registerReferenceMutation\.reset\(\)/,
  );
});

test("unregister rejection is consumed instead of escaping the UI event", () => {
  assert.match(
    agentsView,
    /unregisterReferenceMutation[\s\S]*\.mutateAsync\(reference\)[\s\S]*\.then\([\s\S]*\.catch\(/,
  );
  assert.match(
    agentsView,
    /\.catch\(\(error\) =>[\s\S]*agents\.setActionErrorMessage\(/,
  );
});
