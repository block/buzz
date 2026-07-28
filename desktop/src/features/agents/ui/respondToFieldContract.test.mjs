import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const respondToFieldSource = await readFile(
  new URL("./RespondToField.tsx", import.meta.url),
  "utf8",
);

for (const label of ["Only me (default)", "Selected people", "Anyone"]) {
  test(`respond-to control uses the plain-language label: ${label}`, () => {
    assert.ok(respondToFieldSource.includes(`label: "${label}"`));
  });
}

test("native and persona controls share one option list", () => {
  assert.match(
    respondToFieldSource,
    /<select[\s\S]*RESPOND_TO_OPTIONS\.map\(\(option\) => \([\s\S]*<option/,
  );
});

test("open agent access always renders a persistent warning", () => {
  assert.match(
    respondToFieldSource,
    /mode === "anyone"[\s\S]*data-testid="agent-access-warning"/,
  );
  assert.match(
    respondToFieldSource,
    /Anyone will be able to access the computer or server running this[\s\S]*agent\./,
  );
});

test("primary respond-to copy does not expose implementation jargon", () => {
  const primaryFieldSource = respondToFieldSource.slice(
    respondToFieldSource.indexOf('data-testid="agent-respond-to"'),
    respondToFieldSource.indexOf("const HEX_64_RE"),
  );

  for (const jargon of ["Nostr authors", "!shutdown"]) {
    assert.doesNotMatch(primaryFieldSource, new RegExp(jargon));
  }
});
