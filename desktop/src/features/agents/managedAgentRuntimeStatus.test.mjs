import assert from "node:assert/strict";
import test from "node:test";

import {
  agentCommunityAvailability,
  agentCommunityStatusDetail,
  canonicalRelayUrl,
  connectionTargetsMatch,
  connectionTargetUrl,
  findManagedAgentRuntime,
  managedAgentRuntimeKey,
} from "./managedAgentRuntimeStatus.ts";

const runtime = (overrides = {}) => ({
  pubkey: "aa",
  relayUrl: "wss://relay.example",
  localSetup: true,
  lifecycle: "ready",
  pid: 1,
  error: null,
  logPath: null,
  ...overrides,
});

test("projects every backend lifecycle to the four product labels", () => {
  assert.equal(agentCommunityAvailability(runtime()), "Here");
  for (const lifecycle of ["starting", "listening", "waking"]) {
    assert.equal(agentCommunityAvailability(runtime({ lifecycle })), "Waking");
  }
  for (const lifecycle of ["failed", "stopped"]) {
    assert.equal(
      agentCommunityAvailability(runtime({ lifecycle })),
      "Unavailable",
    );
  }
});

test("backend-authoritative local setup takes precedence", () => {
  assert.equal(
    agentCommunityAvailability(
      runtime({ localSetup: false, lifecycle: "ready" }),
    ),
    "Needs setup on this device",
  );
});

test("unavailable detail distinguishes stopped and failed", () => {
  assert.equal(
    agentCommunityStatusDetail(runtime({ lifecycle: "stopped" })),
    "Stopped by you",
  );
  assert.equal(
    agentCommunityStatusDetail(
      runtime({ lifecycle: "failed", error: "Relay timed out" }),
    ),
    "Relay timed out",
  );
});

test("pair key cannot collide at component boundaries", () => {
  assert.notEqual(
    managedAgentRuntimeKey(runtime({ pubkey: "ab", relayUrl: "c" })),
    managedAgentRuntimeKey(runtime({ pubkey: "a", relayUrl: "bc" })),
  );
  assert.equal(
    managedAgentRuntimeKey(
      runtime({
        pubkey: "AA",
        requestedRelayUrl: "ws://LOCALHOST:80/",
      }),
    ),
    managedAgentRuntimeKey(
      runtime({ pubkey: "aa", requestedRelayUrl: "ws://localhost" }),
    ),
  );
  assert.notEqual(
    managedAgentRuntimeKey(
      runtime({
        relayUrl: "ws://127.0.0.1:3000",
        requestedRelayUrl: "ws://localhost:3000",
      }),
    ),
    managedAgentRuntimeKey(
      runtime({
        relayUrl: "ws://127.0.0.1:3000",
        requestedRelayUrl: "ws://127.0.0.1:3000",
      }),
    ),
  );
});

test("selects one relay without collapsing same-pubkey pairs", () => {
  const runtimes = [
    runtime({ relayUrl: "wss://a.example", lifecycle: "ready" }),
    runtime({ relayUrl: "wss://b.example", lifecycle: "failed" }),
  ];
  assert.equal(
    findManagedAgentRuntime(runtimes, "AA", "wss://b.example")?.lifecycle,
    "failed",
  );
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "wss://c.example"),
    undefined,
  );
});

test("canonicalRelayUrl mirrors the backend pair-key normalization", () => {
  // Loopback folding + default-port and trailing-slash stripping — the
  // standard dev setup that previously broke pair matching.
  assert.equal(canonicalRelayUrl("ws://localhost:3000"), "ws://127.0.0.1:3000");
  assert.equal(
    canonicalRelayUrl("WSS://Relay.Example:443/"),
    "wss://relay.example",
  );
  assert.equal(
    canonicalRelayUrl("ws://relay.example:80/"),
    "ws://relay.example",
  );
  assert.equal(
    canonicalRelayUrl("wss://relay.example/path/"),
    "wss://relay.example/path",
  );
  assert.equal(canonicalRelayUrl("ws://[::1]:3000"), "ws://127.0.0.1:3000");
  assert.equal(canonicalRelayUrl("https://relay.example"), null);
  assert.equal(canonicalRelayUrl("not a url"), null);
  assert.equal(canonicalRelayUrl("wss://alice@relay.example"), null);
  assert.equal(canonicalRelayUrl("wss://relay.example#east"), null);
  assert.equal(canonicalRelayUrl("wss://relay.example#"), null);
  assert.equal(
    canonicalRelayUrl("wss://@relay.example"),
    "wss://relay.example",
  );
  assert.equal(
    canonicalRelayUrl("wss://:@relay.example"),
    "wss://relay.example",
  );
});

test("matches a stored community URL against canonical backend rows", () => {
  const runtimes = [
    runtime({ relayUrl: "ws://127.0.0.1:3000", lifecycle: "ready" }),
  ];
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "ws://localhost:3000")?.lifecycle,
    "ready",
  );
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "ws://localhost:3001"),
    undefined,
  );
});

test("requestedRelayUrl is authoritative: distinct loopback tenants never alias", () => {
  // One agent, one canonical key, child actually dialed to localhost while
  // both loopback communities are configured simultaneously.
  const runtimes = [
    runtime({
      relayUrl: "ws://127.0.0.1:3000",
      requestedRelayUrl: "ws://localhost:3000",
      lifecycle: "ready",
    }),
  ];
  // The community that was actually dialed resolves the runtime...
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "ws://localhost:3000")?.lifecycle,
    "ready",
  );
  // ...the OTHER loopback community must not: its card would otherwise show
  // this agent as running there, and its Stop/Restart action could target
  // the localhost child through the shared canonical key.
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "ws://127.0.0.1:3000"),
    undefined,
  );
});

test("requested-URL matching folds connection-equivalent spellings only", () => {
  const runtimes = [
    runtime({
      relayUrl: "ws://127.0.0.1:80",
      requestedRelayUrl: "ws://localhost:80",
    }),
  ];
  for (const spelling of [
    "ws://LocalHost:80",
    "ws://localhost",
    "ws://localhost/",
  ]) {
    assert.ok(
      findManagedAgentRuntime(runtimes, "aa", spelling),
      `equivalent spelling must match: ${spelling}`,
    );
  }
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "wss://localhost"),
    undefined,
  );
});

test("connectionTargetUrl folds formatting but preserves tenant hosts", () => {
  assert.equal(connectionTargetUrl("ws://LocalHost:80/"), "ws://localhost");
  assert.equal(
    connectionTargetUrl("wss://relay.example."),
    "wss://relay.example",
  );
  assert.notEqual(
    connectionTargetUrl("ws://localhost:3000"),
    connectionTargetUrl("ws://127.0.0.1:3000"),
  );
  assert.equal(connectionTargetUrl("https://relay.example"), null);
  // Query rides along; the root slash folds with or without one.
  assert.equal(
    connectionTargetUrl("ws://relay.example?token=x"),
    connectionTargetUrl("ws://relay.example/?token=x"),
  );
  assert.notEqual(
    connectionTargetUrl("ws://relay.example?token=x"),
    connectionTargetUrl("ws://relay.example?token=y"),
  );
  // The other scheme's default port is a real port, not foldable.
  assert.notEqual(
    connectionTargetUrl("ws://relay.example:443"),
    connectionTargetUrl("ws://relay.example"),
  );
  // Three loopback spellings, three distinct tenants.
  assert.notEqual(
    connectionTargetUrl("ws://[::1]:3000"),
    connectionTargetUrl("ws://localhost:3000"),
  );
  assert.notEqual(
    connectionTargetUrl("ws://[::1]:3000"),
    connectionTargetUrl("ws://127.0.0.1:3000"),
  );
  assert.equal(
    connectionTargetsMatch(" ws://LOCALHOST:80/ ", "ws://localhost"),
    true,
  );
  assert.equal(
    connectionTargetsMatch("ws://localhost:3000", "ws://127.0.0.1:3000"),
    false,
  );
  assert.equal(connectionTargetsMatch(" invalid ", "invalid"), true);
});

test("invalid connection targets keep exact-string fallback instead of aliasing", () => {
  assert.equal(connectionTargetUrl("wss://alice@relay.example"), null);
  assert.equal(connectionTargetUrl("wss://relay.example#east"), null);
  assert.equal(
    connectionTargetsMatch(
      "wss://alice@relay.example",
      "wss://bob@relay.example",
    ),
    false,
  );
  assert.equal(
    connectionTargetsMatch("wss://alice@relay.example", "wss://relay.example"),
    false,
  );
  assert.equal(
    connectionTargetsMatch(
      "wss://relay.example#east",
      "wss://relay.example#west",
    ),
    false,
  );
  assert.equal(
    connectionTargetsMatch("wss://relay.example#east", "wss://relay.example"),
    false,
  );
  assert.equal(
    connectionTargetsMatch(
      "wss://alice@relay.example",
      "wss://alice@relay.example",
    ),
    true,
  );
  assert.equal(
    connectionTargetsMatch(
      " wss://relay.example#east ",
      "wss://relay.example#east",
    ),
    true,
  );
  assert.equal(
    connectionTargetsMatch("ws://LOCALHOST:80/", "ws://localhost"),
    true,
  );
  assert.equal(
    connectionTargetsMatch("ws://localhost:3000", "ws://127.0.0.1:3000"),
    false,
  );
  assert.equal(
    connectionTargetUrl("wss://@relay.example"),
    "wss://relay.example",
  );
  assert.equal(connectionTargetUrl("wss://relay.example#"), null);
  assert.equal(
    connectionTargetsMatch("wss://@relay.example", "wss://relay.example"),
    true,
  );
  assert.equal(
    connectionTargetsMatch("wss://relay.example#", "wss://relay.example"),
    false,
  );
});

test("legacy rows without requestedRelayUrl reject fragment and credential aliases", () => {
  const runtimes = [runtime({ relayUrl: "wss://relay.example" })];
  assert.ok(findManagedAgentRuntime(runtimes, "aa", "wss://relay.example"));
  assert.ok(findManagedAgentRuntime(runtimes, "aa", "wss://@relay.example"));
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "wss://alice@relay.example"),
    undefined,
  );
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "wss://relay.example#east"),
    undefined,
  );
  assert.equal(
    findManagedAgentRuntime(runtimes, "aa", "wss://relay.example#"),
    undefined,
  );
});
