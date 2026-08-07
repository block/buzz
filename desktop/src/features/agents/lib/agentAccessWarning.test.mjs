import assert from "node:assert/strict";
import test from "node:test";

import {
  agentAccessWarningText,
  runLocationForBackend,
  runLocationForRunOn,
} from "./agentAccessWarning.ts";

test("only the modes that share access warn", () => {
  assert.equal(agentAccessWarningText("owner-only", "local"), null);
  assert.ok(agentAccessWarningText("anyone", "local"));
  assert.ok(agentAccessWarningText("allowlist", "local"));
});

test("a local agent names this computer and what is reachable on it", () => {
  assert.equal(
    agentAccessWarningText("anyone", "local"),
    "Anyone can use this agent to access your computer, including files, accounts, and connected tools.",
  );
  assert.equal(
    agentAccessWarningText("allowlist", "local"),
    "Selected people can use this agent to access your computer, including files, accounts, and connected tools.",
  );
});

test("a provider-backed agent names the server, and not the owner's files", () => {
  // A remote host's files aren't the owner's to describe, so the tail narrows
  // to the accounts and tools provisioned there.
  assert.equal(
    agentAccessWarningText("anyone", "remote"),
    "Anyone can use this agent to access the server it runs on, including any accounts and tools available there.",
  );
  assert.equal(
    agentAccessWarningText("allowlist", "remote"),
    "Selected people can use this agent to access the server it runs on, including any accounts and tools available there.",
  );
  assert.doesNotMatch(
    agentAccessWarningText("anyone", "remote"),
    /your computer/,
  );
});

test("an unknown run location reads as local, not as a hedge", () => {
  // "computer or server" names a concept most owners have never been shown:
  // the Run on selector only renders when a buzz-backend-* provider exists.
  for (const unknown of [undefined, null]) {
    assert.equal(
      agentAccessWarningText("anyone", unknown),
      "Anyone can use this agent to access your computer, including files, accounts, and connected tools.",
    );
  }
});

test("every variant leads with the audience and stays jargon-free", () => {
  for (const mode of ["anyone", "allowlist"]) {
    for (const runLocation of [null, "local", "remote"]) {
      const text = agentAccessWarningText(mode, runLocation);
      assert.match(
        text,
        /^(Anyone|Selected people) can use this agent to access/,
      );
      assert.doesNotMatch(text, /respond-to|allowlist|pubkey|Nostr|harness/i);
    }
  }
});

test("runLocationForBackend maps the backend union", () => {
  assert.equal(runLocationForBackend({ type: "local" }), "local");
  assert.equal(
    runLocationForBackend({ type: "provider", id: "blox", config: {} }),
    "remote",
  );
  assert.equal(runLocationForBackend(null), null);
  assert.equal(runLocationForBackend(undefined), null);
});

test("runLocationForRunOn treats a provider id as remote", () => {
  assert.equal(runLocationForRunOn("local"), "local");
  assert.equal(runLocationForRunOn("blox"), "remote");
});

test("runLocationForRunOn treats a blank value as unknown", () => {
  // `runOn` is typed `"local" | string`, so a blank must not read as a
  // provider id and produce the server wording.
  assert.equal(runLocationForRunOn(""), null);
  assert.equal(runLocationForRunOn(null), null);
  assert.equal(runLocationForRunOn(undefined), null);
});
