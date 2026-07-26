#!/usr/bin/env node
// Tier-3 black-box conformance runner for the portable relay boundary.
//
// Runs the shared fixture vectors against any adapter over plain HTTP(S) and
// WebSocket, recording per-check outcomes as portable evidence. The same
// script runs against the laptop adapter and a deployed Cloudflare preview so
// their outcome sequences can be compared verbatim.
//
// Usage:
//   node scripts/tier3-conformance.mjs <http-base-url> <label> [--mode full|recovery] [--out FILE]
//
// `recovery` mode checks only that previously accepted durable history is
// still served exactly (used after an eviction, isolate change, or redeploy).

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { finalizeEvent, generateSecretKey, getPublicKey } from "nostr-tools";

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const specsRoot = join(packageRoot, "..", "..", "specs");
const signedEvent = JSON.parse(
  readFileSync(
    join(specsRoot, "fixtures/local-relay/signed-message.json"),
    "utf8",
  ),
);
const coreFixture = JSON.parse(
  readFileSync(
    join(specsRoot, "fixtures/portable-relay/core-v0.1.json"),
    "utf8",
  ),
);

const [baseArg, label, ...rest] = process.argv.slice(2);
if (!baseArg || !label) {
  console.error(
    "usage: tier3-conformance.mjs <http-base-url> <label> [--mode full|recovery] [--out FILE]",
  );
  process.exit(1);
}
const httpBase = baseArg.replace(/\/$/, "");
const wsBase = httpBase.replace(/^http/, "ws");
const mode = rest.includes("--mode")
  ? rest[rest.indexOf("--mode") + 1]
  : "full";
const outFile = rest.includes("--out") ? rest[rest.indexOf("--out") + 1] : null;
const FRAME_TIMEOUT_MS = 5_000;
const SILENCE_WINDOW_MS = 1_500;

const results = [];
function record(check, pass, detail) {
  results.push({ check, outcome: pass ? "pass" : "fail", detail });
  console.log(
    `${pass ? "PASS" : "FAIL"}  ${check}${detail ? ` — ${detail}` : ""}`,
  );
}

// Envelope exactness means identical fields and values; JSON key order is
// not semantic and differs between adapters' serializers.
function canonical(value) {
  if (Array.isArray(value)) {
    return value.map(canonical);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonical(value[key])]),
    );
  }
  return value;
}
const deepEqual = (a, b) =>
  JSON.stringify(canonical(a)) === JSON.stringify(canonical(b));
const normalize = (event) => JSON.parse(JSON.stringify(event));

async function post(path, body) {
  const response = await fetch(`${httpBase}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  let json = null;
  try {
    json = await response.json();
  } catch {
    // non-JSON bodies count as protocol failures at the call sites
  }
  return { status: response.status, json };
}

function openSocket() {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(`${wsBase}/`);
    const frames = [];
    const waiters = [];
    socket.addEventListener("message", (event) => {
      const frame = JSON.parse(String(event.data));
      const index = waiters.findIndex((waiter) => waiter.match(frame));
      if (index >= 0) {
        const [waiter] = waiters.splice(index, 1);
        clearTimeout(waiter.timer);
        waiter.resolve(frame);
      } else {
        frames.push(frame);
      }
    });
    socket.addEventListener("open", () =>
      resolve({
        socket,
        send: (frame) => socket.send(JSON.stringify(frame)),
        next(match, timeoutMs = FRAME_TIMEOUT_MS) {
          const buffered = frames.findIndex((frame) => match(frame));
          if (buffered >= 0) {
            return Promise.resolve(frames.splice(buffered, 1)[0]);
          }
          return new Promise((resolveFrame, rejectFrame) => {
            const waiter = { match, resolve: resolveFrame };
            waiter.timer = setTimeout(() => {
              waiters.splice(waiters.indexOf(waiter), 1);
              rejectFrame(new Error("timed out waiting for frame"));
            }, timeoutMs);
            waiters.push(waiter);
          });
        },
        silence: (windowMs = SILENCE_WINDOW_MS) =>
          new Promise((resolveSilence) => {
            const timer = setTimeout(() => {
              waiters.splice(
                waiters.findIndex((waiter) => waiter.timer === null),
                1,
              );
              resolveSilence(null);
            }, windowMs);
            waiters.push({
              match: () => true,
              timer: null,
              resolve: (frame) => {
                clearTimeout(timer);
                resolveSilence(frame);
              },
            });
          }),
      }),
    );
    socket.addEventListener("error", () =>
      reject(new Error("WebSocket failed to connect")),
    );
  });
}

function freshEvent(template, secretKey = generateSecretKey()) {
  return {
    secretKey,
    pubkey: getPublicKey(secretKey),
    event: normalize(finalizeEvent({ tags: [], ...template }, secretKey)),
  };
}

async function checkDurableHistory(stage) {
  const query = await post("/query", coreFixture.operations.query.filters);
  record(
    `${stage}: fixture envelope served exactly`,
    query.status === 200 &&
      Array.isArray(query.json) &&
      query.json.some((event) => deepEqual(event, signedEvent)),
    `status ${query.status}`,
  );
  const count = await post("/count", coreFixture.operations.query.filters);
  record(
    `${stage}: count includes durable fixture event`,
    count.status === 200 && count.json?.count >= 1,
    `count ${count.json?.count}`,
  );
  const socket = await openSocket();
  socket.send(["REQ", "t3-history", coreFixture.operations.query.filters[0]]);
  const history = await socket.next(
    (frame) => frame[0] === "EVENT" && frame[1] === "t3-history",
  );
  const eose = await socket.next(
    (frame) => frame[0] === "EOSE" && frame[1] === "t3-history",
  );
  record(
    `${stage}: WebSocket history precedes EOSE`,
    deepEqual(history[2], signedEvent) && eose[1] === "t3-history",
  );
  socket.socket.close(1000, "done");
}

async function runFull() {
  const health = await fetch(`${httpBase}/health`).then((response) =>
    response.json(),
  );
  record(
    "health endpoint responds",
    health.status === "ok",
    JSON.stringify(health),
  );

  const submit = await post("/events", signedEvent);
  record(
    "fixture event accepted as stored",
    submit.status === 200 &&
      submit.json?.accepted ===
        coreFixture.operations.submit.expected.accepted &&
      submit.json?.message === "stored",
    `${submit.status} ${JSON.stringify(submit.json)}`,
  );

  const duplicate = await post("/events", signedEvent);
  record(
    "resubmitted fixture event is duplicate",
    duplicate.status === 200 && duplicate.json?.message === "duplicate",
    duplicate.json?.message,
  );

  await checkDurableHistory("initial");

  const tampered = freshEvent({
    kind: 1,
    created_at: 400,
    content: "attested",
  });
  const forged = { ...tampered.event, content: "tampered after signing" };
  const rejected = await post("/events", forged);
  record(
    "tampered event rejected as invalid",
    // Denial outcomes are normative; message text is adapter-informative.
    rejected.status === 200 && rejected.json?.accepted === false,
    rejected.json?.message,
  );
  const absent = await post("/query", [{ ids: [tampered.event.id] }]);
  record(
    "rejected event absent from durable history",
    absent.status === 200 &&
      Array.isArray(absent.json) &&
      absent.json.length === 0,
  );

  const search = await post("/query", [{ search: "owner-attested" }]);
  record(
    "NIP-50 search fails explicitly",
    search.status === 400,
    `status ${search.status}`,
  );
  const unknownField = await post("/query", [
    { kinds: [1], unknown_extension: 1 },
  ]);
  record(
    "unknown filter field fails closed",
    unknownField.status === 400,
    `status ${unknownField.status}`,
  );

  const replaceKey = generateSecretKey();
  const newer = freshEvent(
    { kind: 10_000, created_at: 200, content: "newer" },
    replaceKey,
  );
  const older = freshEvent(
    { kind: 10_000, created_at: 100, content: "older" },
    replaceKey,
  );
  const newerResult = await post("/events", newer.event);
  const olderResult = await post("/events", older.event);
  const effective = await post("/query", [
    { kinds: [10_000], authors: [newer.pubkey] },
  ]);
  record(
    "replaceable stream keeps deterministic winner",
    newerResult.json?.message === "stored" &&
      olderResult.json?.message === "superseded" &&
      deepEqual(effective.json, [newer.event]),
    `${newerResult.json?.message}/${olderResult.json?.message}`,
  );

  const ephemeral = freshEvent({
    kind: 20_001,
    created_at: 300,
    content: "live only",
  });
  const ephemeralResult = await post("/events", ephemeral.event);
  const ephemeralQuery = await post("/query", [{ ids: [ephemeral.event.id] }]);
  record(
    "ephemeral event is live-only",
    ephemeralResult.json?.message === "ephemeral" &&
      ephemeralQuery.json?.length === 0,
    ephemeralResult.json?.message,
  );

  const live = freshEvent({
    kind: 1,
    created_at: 500,
    content: "tier-3 live delivery",
  });
  const socket = await openSocket();
  socket.send(["REQ", "t3-live", { kinds: [1], authors: [live.pubkey] }]);
  await socket.next((frame) => frame[0] === "EOSE" && frame[1] === "t3-live");
  const submitLive = await post("/events", live.event);
  const liveFrame = await socket.next(
    (frame) => frame[0] === "EVENT" && frame[1] === "t3-live",
  );
  record(
    "live delivery reaches an open subscription",
    submitLive.json?.message === "stored" &&
      deepEqual(liveFrame[2], live.event),
  );

  socket.send(["CLOSE", "t3-live"]);
  const afterClose = freshEvent(
    { kind: 1, created_at: 600, content: "after CLOSE" },
    live.secretKey,
  );
  const silencePromise = socket.silence();
  await post("/events", afterClose.event);
  const strayFrame = await silencePromise;
  record(
    "CLOSE stops later delivery",
    strayFrame === null,
    strayFrame ? JSON.stringify(strayFrame) : "silent",
  );
  socket.socket.close(1000, "done");
}

function nip98Header(secretKey, url, body) {
  const payload = createHash("sha256").update(body).digest("hex");
  const proof = finalizeEvent(
    {
      kind: 27_235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", url],
        ["method", "POST"],
        ["payload", payload],
        ["nonce", crypto.randomUUID()],
      ],
      content: "",
    },
    secretKey,
  );
  return `Nostr ${Buffer.from(JSON.stringify(proof)).toString("base64")}`;
}

async function authedPost(secretKey, path, body, headerOverride = null) {
  const url = `${httpBase}${path}`;
  const serialized = JSON.stringify(body);
  const response = await fetch(url, {
    method: "POST",
    headers: {
      authorization: headerOverride ?? nip98Header(secretKey, url, serialized),
      "content-type": "application/json",
    },
    body: serialized,
  });
  let json = null;
  try {
    json = await response.json();
  } catch {
    // non-JSON bodies count as protocol failures at the call sites
  }
  return { status: response.status, json };
}

async function runIdentity() {
  const author = freshEvent({ kind: 1, created_at: 1, content: "unused" });
  const note = freshEvent(
    { kind: 1, created_at: nowSecs(), content: "identity-bound over HTTP" },
    author.secretKey,
  );

  const anonymous = await post("/events", note.event);
  record(
    "anonymous submit fails closed",
    anonymous.status === 401 &&
      anonymous.json?.code === "authentication_required",
    `status ${anonymous.status} code ${anonymous.json?.code}`,
  );

  const url = `${httpBase}/events`;
  const serialized = JSON.stringify(note.event);
  const header = nip98Header(author.secretKey, url, serialized);
  const stored = await authedPost(
    author.secretKey,
    "/events",
    note.event,
    header,
  );
  record(
    "authenticated author submit is stored",
    stored.status === 200 && stored.json?.message === "stored",
    `${stored.status} ${stored.json?.message}`,
  );

  const replayed = await authedPost(
    author.secretKey,
    "/events",
    note.event,
    header,
  );
  record(
    "replayed proof fails closed",
    replayed.status === 401 && replayed.json?.code === "replay_detected",
    `status ${replayed.status} code ${replayed.json?.code}`,
  );

  const other = freshEvent({ kind: 1, created_at: 1, content: "unused" });
  const mismatched = await authedPost(other.secretKey, "/events", note.event);
  record(
    "mismatched author fails closed",
    mismatched.status === 403 && mismatched.json?.code === "author_mismatch",
    `status ${mismatched.status} code ${mismatched.json?.code}`,
  );

  const authKindEvent = freshEvent(
    {
      kind: 22_242,
      created_at: nowSecs(),
      content: "",
      tags: [
        ["challenge", "c".repeat(64)],
        ["relay", `${wsBase}/`],
      ],
    },
    author.secretKey,
  );
  const scopeDenied = await authedPost(
    author.secretKey,
    "/events",
    authKindEvent.event,
  );
  record(
    "authentication kinds are denied direct append",
    scopeDenied.status === 403 && scopeDenied.json?.code === "scope_denied",
    `status ${scopeDenied.status} code ${scopeDenied.json?.code}`,
  );
  const journal = await authedPost(author.secretKey, "/query", [
    { kinds: [22_242, 27_235], authors: [author.pubkey] },
  ]);
  record(
    "authentication events never enter the journal",
    journal.status === 200 &&
      Array.isArray(journal.json) &&
      journal.json.length === 0,
  );

  const recipient = freshEvent({ kind: 1, created_at: 1, content: "unused" });
  const attacker = freshEvent({ kind: 1, created_at: 1, content: "unused" });
  const giftWrap = freshEvent({
    kind: 1_059,
    created_at: nowSecs(),
    content: "ciphertext",
    tags: [["p", recipient.pubkey]],
  });
  const wrapStored = await authedPost(
    author.secretKey,
    "/events",
    giftWrap.event,
  );
  const wrapFilters = [{ ids: [giftWrap.event.id] }];
  const recipientQuery = await authedPost(
    recipient.secretKey,
    "/query",
    wrapFilters,
  );
  const attackerQuery = await authedPost(
    attacker.secretKey,
    "/query",
    wrapFilters,
  );
  record(
    "gift-wrap disclosure binds to the tagged recipient",
    wrapStored.status === 200 &&
      deepEqual(recipientQuery.json, [giftWrap.event]) &&
      Array.isArray(attackerQuery.json) &&
      attackerQuery.json.length === 0,
  );
  const recipientCount = await authedPost(
    recipient.secretKey,
    "/count",
    wrapFilters,
  );
  const attackerCount = await authedPost(
    attacker.secretKey,
    "/count",
    wrapFilters,
  );
  record(
    "count applies the same disclosure rule",
    recipientCount.json?.count === 1 && attackerCount.json?.count === 0,
    `${recipientCount.json?.count}/${attackerCount.json?.count}`,
  );

  const socket = await openSocket();
  const challengeFrame = await socket.next((frame) => frame[0] === "AUTH");
  record(
    "WebSocket issues an AUTH challenge",
    typeof challengeFrame[1] === "string" && challengeFrame[1].length > 0,
  );

  const wsKeys = freshEvent({ kind: 1, created_at: 1, content: "unused" });
  const early = freshEvent(
    { kind: 1, created_at: nowSecs(), content: "too early" },
    wsKeys.secretKey,
  );
  socket.send(["EVENT", early.event]);
  const earlyDenied = await socket.next(
    (frame) => frame[0] === "OK" && frame[1] === early.event.id,
  );
  record(
    "unauthenticated WebSocket EVENT fails closed",
    earlyDenied[2] === false && earlyDenied[3] === "authentication_required",
    String(earlyDenied[3]),
  );
  socket.send(["REQ", "t3-id-early", { kinds: [1] }]);
  const earlyClosed = await socket.next(
    (frame) => frame[0] === "CLOSED" && frame[1] === "t3-id-early",
  );
  record(
    "unauthenticated WebSocket REQ fails closed",
    String(earlyClosed[2]).startsWith("authentication_required"),
    String(earlyClosed[2]),
  );

  const authEvent = freshEvent(
    {
      kind: 22_242,
      created_at: nowSecs(),
      content: "",
      tags: [
        ["challenge", challengeFrame[1]],
        ["relay", `${wsBase}/`],
      ],
    },
    wsKeys.secretKey,
  );
  socket.send(["AUTH", authEvent.event]);
  const authOk = await socket.next(
    (frame) => frame[0] === "OK" && frame[1] === authEvent.event.id,
  );
  record(
    "NIP-42 proof authenticates the WebSocket",
    authOk[2] === true && authOk[3] === "authenticated",
    String(authOk[3]),
  );

  socket.send(["REQ", "t3-id-live", { kinds: [1], authors: [wsKeys.pubkey] }]);
  await socket.next(
    (frame) => frame[0] === "EOSE" && frame[1] === "t3-id-live",
  );
  const liveNote = freshEvent(
    { kind: 1, created_at: nowSecs(), content: "identity-bound live delivery" },
    wsKeys.secretKey,
  );
  const liveStored = await authedPost(
    wsKeys.secretKey,
    "/events",
    liveNote.event,
  );
  const liveFrame = await socket.next(
    (frame) => frame[0] === "EVENT" && frame[1] === "t3-id-live",
  );
  record(
    "authenticated live delivery works end to end",
    liveStored.status === 200 && deepEqual(liveFrame[2], liveNote.event),
  );
  socket.socket.close(1000, "done");
}

function nowSecs() {
  return Math.floor(Date.now() / 1000);
}

if (mode === "full") {
  await runFull();
} else if (mode === "identity") {
  await runIdentity();
} else {
  await checkDurableHistory("recovery");
}

const evidence = {
  capability: "portable-relay-cloudflare-v0.1",
  profile:
    mode === "identity"
      ? "portable-relay-identity-v0.1"
      : "portable-relay-core-v0.1",
  adapter: label,
  base_url: httpBase,
  mode,
  fixtures: [
    "local-relay/signed-message.json",
    "portable-relay/core-v0.1.json",
  ],
  recorded_at: new Date().toISOString(),
  results,
  pass: results.every((result) => result.outcome === "pass"),
};
if (outFile) {
  writeFileSync(outFile, `${JSON.stringify(evidence, null, 2)}\n`);
}
console.log(
  `\n${evidence.pass ? "TIER PASS" : "TIER FAIL"}: ${label} (${mode}) — ${results.length} checks`,
);
process.exit(evidence.pass ? 0 : 1);
