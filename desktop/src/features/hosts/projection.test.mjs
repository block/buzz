import assert from "node:assert/strict";
import test from "node:test";
import { fixture } from "./hostTestFixtures.mjs";
import { validateHostReport } from "./reportValidation.ts";

for (const label of ["registration", "report"]) {
  test(`newer invalid ${label} does not erase valid remembered state`, async () => {
    const f = fixture();
    const first = await f.run();
    f.writes.length = 0;
    const valid = f.events[label === "registration" ? 0 : 1];
    f.events.push({
      ...valid,
      id: "f".repeat(64),
      created_at: 101,
      invalid: true,
    });
    const operation = label === "registration" ? "inspect" : "decode";
    const original = f.bridge[operation];
    f.bridge[operation] = async (...args) => {
      if (args.at(-1).invalid) throw new Error("invalid encrypted record");
      return original(...args);
    };
    f.setNow(110);
    const result = await f.run();
    assert.equal(result.rows[0].registration.id, first.rows[0].registration.id);
    assert.equal(result.rows[0].event.id, first.rows[0].event.id);
    assert.equal(f.writes.length, 0);
  });
}

test("latest valid registration uses timestamp DESC, id ASC independent of arrival", async () => {
  const f = fixture();
  const registration = await f.bridge.registration();
  const low = { ...registration, id: "1".repeat(64), created_at: 101 };
  const high = { ...registration, id: "2".repeat(64), created_at: 101 };
  f.events.push(high, registration, low);
  f.events.push(await f.bridge.report(low));
  const result = await f.run();
  assert.equal(result.rows[0].registration.id, low.id);
  assert.equal(f.writes.length, 0);
});

test("latest valid report uses timestamp DESC, id ASC and keeps expired capabilities", async () => {
  const f = fixture();
  const registration = await f.bridge.registration();
  const report = await f.bridge.report(registration);
  const low = { ...report, id: "1".repeat(64), created_at: 101 };
  const high = { ...report, id: "2".repeat(64), created_at: 101 };
  f.events.push(registration, high, report, low);
  let result = await f.run();
  assert.equal(result.rows[0].event.id, low.id);
  // A remembered nonlocal host remains durable when its lease expires.
  f.bridge.local = async () => ({ host: "c".repeat(64), report: f.payload });
  const other = {
    ...registration,
    id: "3".repeat(64),
    tags: registration.tags.map((t) =>
      t[0] === "x" ? ["x", "c".repeat(64)] : t,
    ),
  };
  f.events.push(other);
  f.bridge.inspect = async (event) => event.tags.find((t) => t[0] === "x")[1];
  // Keep the local fixture's renewed lease fresh so only remembered state is tested.
  const otherReport = {
    ...report,
    pubkey: "c".repeat(64),
    id: "4".repeat(64),
    created_at: 300,
    tags: report.tags.map((t) =>
      t[0] === "x"
        ? ["x", "c".repeat(64)]
        : t[0] === "e"
          ? ["e", other.id]
          : t[0] === "valid_until"
            ? ["valid_until", "480"]
            : t,
    ),
  };
  f.events.push(otherReport);
  f.setNow(300);
  result = await f.run();
  assert.equal(
    result.rows.find((row) => row.host === "b".repeat(64)).event.id,
    low.id,
  );
  assert.equal(f.writes.length, 0);
});

test("incomplete report history blocks all event construction/publication", async () => {
  const f = fixture();
  const registration = await f.bridge.registration();
  f.events.push(
    registration,
    ...Array(1000).fill(await f.bridge.report(registration)),
  );
  f.bridge.registration = f.bridge.report = async () =>
    assert.fail("construction after incomplete history");
  await assert.rejects(f.run(), /cursor did not advance/);
  assert.equal(f.writes.length, 0);
});

test("unreadable report history is not mistaken for no report", async () => {
  const f = fixture();
  await f.run();
  f.writes.length = 0;
  f.bridge.decode = async () => {
    throw new Error("decode failed");
  };
  await assert.rejects(f.run(), /Cannot establish host capabilities/);
  assert.equal(f.writes.length, 0);
});

const invalidPayloads = [
  [
    "unknown availability",
    (r) => {
      r.runtimes[0].availability = "ready";
    },
  ],
  [
    "unknown auth status",
    (r) => {
      r.runtimes[0].auth_status = "ready";
    },
  ],
  [
    "duplicate runtime",
    (r) => {
      r.runtimes.push({ ...r.runtimes[0] });
    },
  ],
  [
    "unknown field",
    (r) => {
      r.diagnostic = "not allowed";
    },
  ],
  [
    "runtime extra field",
    (r) => {
      r.runtimes[0].path = "not allowed";
    },
  ],
  [
    "oversized text bytes",
    (r) => {
      r.name = "é".repeat(129);
    },
  ],
  [
    "control text",
    (r) => {
      r.name = "bad\u0085text";
    },
  ],
  [
    "empty text",
    (r) => {
      r.name = "";
    },
  ],
  [
    "unsupported version",
    (r) => {
      r.v = 4;
    },
  ],
  [
    "start enabled",
    (r) => {
      r.accepts_start = true;
    },
  ],
  [
    "oversized catalog",
    (r) => {
      r.runtimes = Array(129).fill(r.runtimes[0]);
    },
  ],
  [
    "missing runtime id",
    (r) => {
      delete r.runtimes[0].id;
    },
  ],
];
for (const [name, mutate] of invalidPayloads) {
  test(`${name} is skipped in history and cannot be published from discovery`, async () => {
    const f = fixture();
    await f.run();
    f.writes.length = 0;
    const invalid = structuredClone(f.events[1]);
    invalid.decoded = structuredClone(f.decoded.get(invalid.id));
    invalid.id = "f".repeat(64);
    invalid.created_at++;
    mutate(invalid.decoded);
    assert.throws(() => validateHostReport(invalid.decoded), /Invalid host/);
    f.events.push(invalid);
    await f.run();
    assert.equal(f.writes.length, 0);
    mutate(f.payload);
    await assert.rejects(f.run(), /Invalid host/);
    assert.equal(f.writes.length, 0);
  });
}

test("all real native runtime statuses and empty catalogs are supported", () => {
  const f = fixture();
  for (const availability of [
    "available",
    "adapter_missing",
    "adapter_outdated",
    "cli_missing",
    "not_installed",
  ]) {
    for (const auth_status of [
      "logged_in",
      "logged_out",
      "config_invalid",
      "not_applicable",
      "unknown",
    ]) {
      f.payload.runtimes[0] = {
        id: "one",
        label: "One",
        availability,
        auth_status,
      };
      validateHostReport(f.payload);
    }
  }
  f.payload.runtimes = [];
  validateHostReport(f.payload);
});

test("a newer duplicate registration reuses unchanged capabilities bound to the older registration", async () => {
  const f = fixture();
  const first = await f.run();
  f.writes.length = 0;
  const newer = { ...f.events[0], id: "f".repeat(64), created_at: 101 };
  f.events.push(newer);
  f.setNow(110);
  const result = await f.run();
  assert.equal(result.rows[0].registration.id, newer.id);
  assert.equal(result.rows[0].event.id, first.rows[0].event.id);
  assert.equal(f.writes.length, 0);
});

test("a report with unknown registration cannot supersede a verified host report", async () => {
  const f = fixture();
  const first = await f.run();
  f.writes.length = 0;
  const invalid = {
    ...f.events[1],
    id: "f".repeat(64),
    created_at: 101,
    tags: f.events[1].tags.map((tag) =>
      tag[0] === "e" ? ["e", "unknown"] : tag,
    ),
  };
  f.events.push(invalid);
  const result = await f.run();
  assert.equal(result.rows[0].event.id, first.rows[0].event.id);
  assert.equal(f.writes.length, 0);
});
