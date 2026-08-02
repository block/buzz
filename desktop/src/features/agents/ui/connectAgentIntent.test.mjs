import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitConnectAgent,
  connectAgentPayload,
  emptyConnectAgentDraft,
  harnessOptions,
  missingBuzzCli,
  nameInputMessage,
  pubkeyInputMessage,
  reachabilityLabel,
  verifyPubkeyInput,
} from "./connectAgentIntent.ts";

const HEX = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
const NPUB = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";

function draft(overrides = {}) {
  return {
    ...emptyConnectAgentDraft,
    host: "workstation",
    pubkey: HEX,
    name: "Scout",
    ...overrides,
  };
}

function probe(overrides = {}) {
  return {
    host: "workstation",
    ok: true,
    durationMs: 900,
    harnesses: [],
    buzzCliPath: "/Users/alice/.local/bin/buzz",
    ...overrides,
  };
}

test("both pubkey forms a user actually has on hand are accepted", () => {
  assert.equal(verifyPubkeyInput(HEX).kind, "ok");
  assert.equal(verifyPubkeyInput(NPUB).kind, "ok");
  assert.equal(verifyPubkeyInput(HEX.toUpperCase()).kind, "ok");
  assert.equal(verifyPubkeyInput(`  ${NPUB}  `).kind, "ok");
});

test("a pasted secret key is called out as such, not as invalid input", () => {
  // A user who pastes an nsec has made a serious mistake. "Invalid pubkey"
  // would not tell them what it was, and they would try again with the same
  // secret.
  assert.equal(verifyPubkeyInput("nsec1abcdef").kind, "secret");
  const message = pubkeyInputMessage("nsec1abcdef");
  assert.match(message, /secret key/i);
  assert.match(message, /never leave/i);
});

test("malformed pubkeys are rejected", () => {
  for (const bad of [
    "not-a-key",
    "npub1short",
    HEX.slice(0, 63),
    `${HEX}f`,
    // Bech32 excludes 1, b, i, and o — a lookalike must not pass.
    `npub1${"b".repeat(58)}`,
  ]) {
    assert.equal(verifyPubkeyInput(bad).kind, "invalid", bad);
  }
});

test("an empty pubkey is silent, not an error", () => {
  // Nothing typed yet is not a mistake; showing red text on an untouched field
  // trains users to ignore it.
  assert.equal(verifyPubkeyInput("").kind, "empty");
  assert.equal(pubkeyInputMessage(""), null);
  assert.equal(pubkeyInputMessage("   "), null);
});

test("names are bounded and the bound is stated", () => {
  assert.equal(nameInputMessage("Scout"), null);
  assert.equal(nameInputMessage(""), null);
  assert.equal(nameInputMessage("n".repeat(64)), null);
  assert.match(nameInputMessage("n".repeat(65)), /64 characters/);
});

test("submit requires a host, a well-formed pubkey, and a name", () => {
  assert.equal(canSubmitConnectAgent(draft()), true);
  assert.equal(canSubmitConnectAgent(draft({ host: "" })), false);
  assert.equal(canSubmitConnectAgent(draft({ host: "   " })), false);
  assert.equal(canSubmitConnectAgent(draft({ pubkey: "nope" })), false);
  assert.equal(canSubmitConnectAgent(draft({ name: "" })), false);
  assert.equal(canSubmitConnectAgent(draft({ name: "n".repeat(65) })), false);
});

test("submit does not require a reachable host", () => {
  // A machine that is asleep, off the VPN, or mid-reboot is still an agent host
  // the user wants recorded. Gating on reachability would break the feature
  // exactly during setup.
  assert.equal(canSubmitConnectAgent(draft({ probe: null })), true);
  assert.equal(
    canSubmitConnectAgent(
      draft({ probe: probe({ ok: false, errorKind: "unreachable" }) }),
    ),
    true,
  );
});

test("submit is blocked while a probe is in flight", () => {
  // The probe fills the harness options; submitting mid-probe would record a
  // null harness the user was about to pick.
  assert.equal(canSubmitConnectAgent(draft({ isProbing: true })), false);
});

test("the payload trims and omits an unset harness", () => {
  assert.deepEqual(
    connectAgentPayload(draft({ host: " workstation ", name: "  Scout  " })),
    { host: "workstation", pubkey: HEX, name: "Scout", harness: null },
  );
  assert.deepEqual(
    connectAgentPayload(draft({ harness: "claude" })).harness,
    "claude",
  );
  assert.equal(connectAgentPayload(draft({ harness: "   " })).harness, null);
});

test("an unsubmittable draft yields no payload", () => {
  assert.equal(connectAgentPayload(draft({ pubkey: "" })), null);
});

test("only ready harnesses are offered", () => {
  // An ACP adapter whose vendor CLI is missing starts and then fails at first
  // use. Offering it would record something known-broken as the agent's
  // harness.
  const options = harnessOptions(
    probe({
      harnesses: [
        { id: "claude", label: "Claude Code", ready: true },
        { id: "codex", label: "Codex", ready: false },
      ],
    }),
  );
  assert.deepEqual(
    options.map((harness) => harness.id),
    ["claude"],
  );
});

test("a failed or absent probe offers no harnesses", () => {
  assert.deepEqual(harnessOptions(null), []);
  assert.deepEqual(
    harnessOptions(
      probe({
        ok: false,
        errorKind: "password_required",
        harnesses: [{ id: "claude", label: "Claude Code", ready: true }],
      }),
    ),
    [],
  );
});

test("a missing buzz CLI is flagged only once the probe succeeded", () => {
  // Without the CLI the agent cannot reach the relay at all, so it is the one
  // warning worth surfacing — but an unreachable host has not told us anything
  // about its CLI, and claiming it is missing would be a fabrication.
  assert.equal(missingBuzzCli(probe({ buzzCliPath: null })), true);
  assert.equal(missingBuzzCli(probe()), false);
  assert.equal(missingBuzzCli(null), false);
  assert.equal(missingBuzzCli(probe({ ok: false, buzzCliPath: null })), false);
});

test("a failed probe is labelled by cause, not as unreachable", () => {
  // "machine unreachable" is wrong for every classified kind except one — the
  // host answered in all the others. The host-key case matters most: Buzz probes
  // with strict checking and never writes known_hosts, so this label is the only
  // prompt telling the user to go review a fingerprint.
  const label = (errorKind) =>
    reachabilityLabel({
      host: "workstation",
      ok: false,
      durationMs: 1,
      errorKind,
      harnesses: [],
    });

  assert.equal(label("host_key_problem"), "host key not trusted");
  assert.equal(label("truncated"), "probe incomplete \u00b7 retry");
  assert.equal(label("password_required"), "needs an ssh key");
  assert.equal(label("timed_out"), "probe timed out");
  assert.equal(label("unreachable"), "machine unreachable");

  // Only `unreachable` may claim the machine could not be reached.
  for (const kind of [
    "host_key_problem",
    "truncated",
    "password_required",
    "timed_out",
  ]) {
    assert.ok(
      !label(kind).includes("unreachable"),
      `${kind} must not be reported as unreachable`,
    );
  }
});

test("an unclassified probe failure does not invent a cause", () => {
  const label = reachabilityLabel({
    host: "workstation",
    ok: false,
    durationMs: 1,
    errorKind: null,
    harnesses: [],
  });
  assert.equal(label, "probe failed");
});
