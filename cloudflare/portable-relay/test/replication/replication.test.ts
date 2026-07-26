import { SELF } from "cloudflare:test";
import { finalizeEvent, generateSecretKey } from "nostr-tools";
import { describe, expect, it } from "vitest";
import {
  hexToBytes,
  TEST_PEER_SECRET_HEX,
  TEST_REPLICATION_SOURCE,
} from "./peer-fixture";

const PEER_SECRET = hexToBytes(TEST_PEER_SECRET_HEX);

async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    bytes.buffer as ArrayBuffer,
  );
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function nip98Header(
  secretKey: Uint8Array,
  url: string,
  body: string,
): Promise<string> {
  const proof = finalizeEvent(
    {
      kind: 27_235,
      created_at: Math.floor(Date.now() / 1000),
      tags: [
        ["u", url],
        ["method", "POST"],
        ["payload", await sha256Hex(new TextEncoder().encode(body))],
        ["nonce", crypto.randomUUID()],
      ],
      content: "",
    },
    secretKey,
  );
  return `Nostr ${btoa(JSON.stringify(proof))}`;
}

async function pushRecords(
  origin: string,
  records: unknown[],
  secretKey: Uint8Array = PEER_SECRET,
): Promise<Response> {
  const url = `${origin}/replication`;
  const body = JSON.stringify(records);
  return SELF.fetch(url, {
    method: "POST",
    headers: {
      authorization: await nip98Header(secretKey, url, body),
      "content-type": "application/json",
    },
    body,
  });
}

function signedNote(content: string, secretKey = generateSecretKey()) {
  return JSON.parse(
    JSON.stringify(
      finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [],
          content,
        },
        secretKey,
      ),
    ),
  );
}

function record(
  cursor: string,
  event: unknown,
  source = TEST_REPLICATION_SOURCE,
) {
  return { source, cursor, event };
}

describe("replication sink", () => {
  it("ingests an ordered batch from the configured peer idempotently", async () => {
    const origin = "https://repl-happy.example";
    const first = signedNote("replicated one");
    const second = signedNote("replicated two");
    const batch = [record("local:1", first), record("local:2", second)];

    const response = await pushRecords(origin, batch);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual([
      {
        source: TEST_REPLICATION_SOURCE,
        cursor: "local:1",
        event_id: first.id,
        outcome: { status: "stored" },
      },
      {
        source: TEST_REPLICATION_SOURCE,
        cursor: "local:2",
        event_id: second.id,
        outcome: { status: "stored" },
      },
    ]);

    const query = await SELF.fetch(`${origin}/query`, {
      method: "POST",
      body: JSON.stringify([{ ids: [first.id, second.id] }]),
    });
    const events = (await query.json()) as { id: string }[];
    expect(events.map((event) => event.id).sort()).toEqual(
      [first.id, second.id].sort(),
    );

    const again = await pushRecords(origin, batch);
    const receipts = (await again.json()) as { outcome: { status: string } }[];
    expect(receipts.map((receipt) => receipt.outcome.status)).toEqual([
      "duplicate",
      "duplicate",
    ]);
  });

  it("denies unbound sources and mismatched peer keys", async () => {
    const origin = "https://repl-denied.example";
    const note = signedNote("must not land");

    const unbound = await pushRecords(origin, [
      record("other:1", note, "unknown-source/stream"),
    ]);
    expect(unbound.status).toBe(403);
    expect(await unbound.json()).toMatchObject({ code: "peer_unbound" });

    const stranger = await pushRecords(
      origin,
      [record("local:1", note)],
      generateSecretKey(),
    );
    expect(stranger.status).toBe(403);
    expect(await stranger.json()).toMatchObject({ code: "source_mismatch" });

    const anonymous = await SELF.fetch(`${origin}/replication`, {
      method: "POST",
      body: JSON.stringify([record("local:1", note)]),
    });
    expect(anonymous.status).toBe(401);
    expect(await anonymous.json()).toMatchObject({
      code: "authentication_required",
    });

    const query = await SELF.fetch(`${origin}/query`, {
      method: "POST",
      body: JSON.stringify([{ ids: [note.id] }]),
    });
    expect(await query.json()).toEqual([]);
  });

  it("rejects invalid records without halting earlier durable progress", async () => {
    const origin = "https://repl-rejects.example";
    const good = signedNote("valid before the bad record");
    const tampered = { ...signedNote("original"), content: "tampered" };
    const after = signedNote("never reached");

    const response = await pushRecords(origin, [
      record("local:1", good),
      record("local:2", tampered),
      record("local:3", after),
    ]);
    const receipts = (await response.json()) as {
      outcome: { status: string };
    }[];
    expect(receipts.map((receipt) => receipt.outcome.status)).toEqual([
      "stored",
      "rejected",
    ]);

    const query = await SELF.fetch(`${origin}/query`, {
      method: "POST",
      body: JSON.stringify([{ ids: [good.id, tampered.id, after.id] }]),
    });
    const events = (await query.json()) as { id: string }[];
    expect(events.map((event) => event.id)).toEqual([good.id]);
  });

  it("keeps ephemeral and authentication events out of replication", async () => {
    const origin = "https://repl-scope.example";
    const keys = generateSecretKey();
    const ephemeral = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: 20_001,
            created_at: Math.floor(Date.now() / 1000),
            tags: [],
            content: "live only",
          },
          keys,
        ),
      ),
    );
    const authEvent = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: 22_242,
            created_at: Math.floor(Date.now() / 1000),
            tags: [
              ["challenge", "c".repeat(64)],
              ["relay", "wss://repl-scope.example/"],
            ],
            content: "",
          },
          keys,
        ),
      ),
    );

    const ephemeralPush = await pushRecords(origin, [
      record("local:1", ephemeral),
    ]);
    expect(await ephemeralPush.json()).toMatchObject([
      {
        outcome: {
          status: "rejected",
          reason: "ephemeral events are not part of durable replication",
        },
      },
    ]);

    const authPush = await pushRecords(origin, [record("local:1", authEvent)]);
    expect(await authPush.json()).toMatchObject([
      { outcome: { status: "rejected" } },
    ]);
  });
});

describe("replication read (rendezvous)", () => {
  const READER = hexToBytes(
    "2222222222222222222222222222222222222222222222222222222222222222",
  );

  async function readStream(
    origin: string,
    source: string,
    cursor: string | null,
    secretKey: Uint8Array = READER,
  ): Promise<Response> {
    const url = `${origin}/replication/read`;
    const body = JSON.stringify({ source, cursor, limit: 100 });
    return SELF.fetch(url, {
      method: "POST",
      headers: {
        authorization: await nip98Header(secretKey, url, body),
        "content-type": "application/json",
      },
      body,
    });
  }

  it("lets an authorized reader drain custodied records with cursor resume", async () => {
    const origin = "https://rendezvous-drain.example";
    const first = signedNote("custodied one");
    const second = signedNote("custodied two");
    await pushRecords(origin, [
      record("local:1", first),
      record("local:2", second),
    ]);

    const page = await readStream(origin, TEST_REPLICATION_SOURCE, null);
    expect(page.status).toBe(200);
    const batch = (await page.json()) as {
      records: { source: string; cursor: string; event: { id: string } }[];
      next_cursor: string;
      caught_up: boolean;
    };
    expect(batch.records.map((r) => r.event.id)).toEqual([first.id, second.id]);
    expect(batch.records[0].source).toBe(TEST_REPLICATION_SOURCE);
    expect(batch.records[0].cursor).toMatch(/^cf-sqlite-v1:/);
    expect(batch.caught_up).toBe(true);

    const resumed = await readStream(
      origin,
      TEST_REPLICATION_SOURCE,
      batch.next_cursor,
    );
    const empty = (await resumed.json()) as {
      records: unknown[];
      caught_up: boolean;
    };
    expect(empty.records).toEqual([]);
    expect(empty.caught_up).toBe(true);
  });

  it("applies export filters and renames streams independently of ingest", async () => {
    const origin = "https://rendezvous-filter.example";
    const note = signedNote("a note");
    const head = JSON.parse(
      JSON.stringify(
        finalizeEvent(
          {
            kind: 30_078,
            created_at: Math.floor(Date.now() / 1000),
            tags: [["d", "demo"]],
            content: "a head",
          },
          generateSecretKey(),
        ),
      ),
    );
    await pushRecords(origin, [
      record("local:1", note),
      record("local:2", head),
    ]);

    const page = await readStream(origin, "rendezvous/notes-only", null);
    const batch = (await page.json()) as {
      records: { event: { id: string } }[];
      caught_up: boolean;
    };
    expect(batch.records.map((r) => r.event.id)).toEqual([note.id]);
    expect(batch.caught_up).toBe(true);
  });

  it("fails closed for unknown streams, strangers, and the write-side peer", async () => {
    const origin = "https://rendezvous-denied.example";
    await pushRecords(origin, [record("local:1", signedNote("custodied"))]);

    const unknown = await readStream(origin, "unknown/stream", null);
    expect(unknown.status).toBe(403);
    expect(await unknown.json()).toMatchObject({ code: "peer_unbound" });

    const stranger = await readStream(
      origin,
      TEST_REPLICATION_SOURCE,
      null,
      generateSecretKey(),
    );
    expect(stranger.status).toBe(403);
    expect(await stranger.json()).toMatchObject({ code: "source_mismatch" });

    // The write-side peer key is not automatically a reader.
    const writer = await readStream(
      origin,
      TEST_REPLICATION_SOURCE,
      null,
      PEER_SECRET,
    );
    expect(writer.status).toBe(403);
    expect(await writer.json()).toMatchObject({ code: "source_mismatch" });
  });
});
