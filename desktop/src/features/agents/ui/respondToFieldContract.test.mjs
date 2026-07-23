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

test("open agent access always renders a persistent warning", () => {
  assert.match(
    respondToFieldSource,
    /mode === "anyone"[\s\S]*data-testid="agent-access-warning"/,
  );
  assert.match(
    respondToFieldSource,
    /Anyone can send instructions to this agent\./,
  );
  assert.match(
    respondToFieldSource,
    /It may use files,[\s\S]*accounts, and tools it can access on the computer or server where it[\s\S]*runs\./,
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
