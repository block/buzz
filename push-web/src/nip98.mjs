// NIP-98 HTTP auth verification for subscription registration. Proves the caller controls the pubkey (via their signer);
// the server never sees or needs a private key.
import { createHash } from "node:crypto";
import { verifyEvent } from "nostr-tools/pure";

const seenNonces = new Map(); // nonce/id -> expiry (replay protection)

export function verifyNip98(header, { method, url, body, now = Math.floor(Date.now() / 1000), maxSkew = 60 }) {
  if (typeof header !== "string" || !header.startsWith("Nostr ")) throw new Error("missing Nostr authorization");
  let ev;
  try {
    ev = JSON.parse(Buffer.from(header.slice(6), "base64").toString("utf8"));
  } catch {
    throw new Error("malformed authorization");
  }
  if (ev?.kind !== 27235) throw new Error("wrong kind");
  if (Math.abs(now - Number(ev.created_at)) > maxSkew) throw new Error("stale authorization");
  const tag = (k) => ev.tags?.find((t) => t[0] === k)?.[1];
  if (tag("u") !== url) throw new Error("url mismatch");
  if ((tag("method") || "").toUpperCase() !== method.toUpperCase()) throw new Error("method mismatch");
  const hash = createHash("sha256").update(body ?? Buffer.alloc(0)).digest("hex");
  if (tag("payload") !== hash) throw new Error("payload mismatch");
  if (!verifyEvent(ev)) throw new Error("bad signature");
  for (const [k, exp] of seenNonces) if (exp < now) seenNonces.delete(k);
  if (seenNonces.has(ev.id)) throw new Error("replayed authorization");
  seenNonces.set(ev.id, now + maxSkew * 2);
  return ev.pubkey;
}
