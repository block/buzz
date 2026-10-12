// Node-side "other client" for e2e: NIP-98 HTTP against the local relay (separate identity from the page under test).
import { createHash, randomUUID } from "node:crypto";
import { finalizeEvent, getPublicKey } from "nostr-tools";
const hex = (h) => new Uint8Array(Buffer.from(h, "hex"));
export function client(base, skHex) {
  const sk = hex(skHex);
  const pubkey = getPublicKey(sk);
  async function post(path, body) {
    const url = `${base}${path}`;
    const bytes = Buffer.from(JSON.stringify(body));
    const ev = finalizeEvent(
      {
        kind: 27235,
        content: "",
        created_at: Math.floor(Date.now() / 1000),
        tags: [
          ["u", url],
          ["method", "POST"],
          ["payload", createHash("sha256").update(bytes).digest("hex")],
          ["nonce", randomUUID()],
        ],
      },
      sk,
    );
    const res = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization:
          "Nostr " + Buffer.from(JSON.stringify(ev)).toString("base64"),
      },
      body: bytes,
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`${path} ${res.status}: ${text}`);
    return JSON.parse(text);
  }
  return {
    pubkey,
    query: (filters) => post("/query", filters),
    publish: (kind, tags, content = "") =>
      post(
        "/events",
        finalizeEvent(
          { kind, content, tags, created_at: Math.floor(Date.now() / 1000) },
          sk,
        ),
      ),
  };
}
