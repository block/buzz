// NIP-OA owner attestation: port of buzz-sdk nip_oa::{parse_auth_tag, verify_auth_tag} and desktop
// nostr_convert::profile_valid_oa_owner_pubkey. An agent's kind:0 carries ["auth", owner, conditions, sig] where sig is the
// owner's BIP-340 signature over sha256("nostr:agent-auth:<agent>:<conditions>").
import { schnorr } from "@noble/curves/secp256k1.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { hexToBytes } from "@noble/hashes/utils.js";
import { verifyEvent } from "nostr-tools/pure";
import type { NostrEvent } from "./nostrTypes";

const LHEX64 = /^[0-9a-f]{64}$/;
const LHEX128 = /^[0-9a-f]{128}$/;

function canonicalDecimal(s: string, max: number): boolean {
  if (!s || (s.length > 1 && s[0] === "0") || !/^\d+$/.test(s)) return false;
  const n = Number(s);
  return Number.isSafeInteger(n) && n <= max;
}
export function validConditions(c: string): boolean {
  if (!c) return true;
  if (/\s/.test(c)) return false;
  return c.split("&").every((cl) => {
    if (cl.startsWith("kind=")) return canonicalDecimal(cl.slice(5), 65535);
    if (cl.startsWith("created_at<")) return canonicalDecimal(cl.slice(11), 4294967295);
    if (cl.startsWith("created_at>")) return canonicalDecimal(cl.slice(11), 4294967295);
    return false;
  });
}

/** Owner pubkey if `tag` is a structurally valid auth tag whose signature verifies for `agentPubkey`, else null. */
export function verifyAuthTag(tag: unknown, agentPubkey: string): string | null {
  if (!Array.isArray(tag) || tag.length !== 4 || tag.some((x) => typeof x !== "string")) return null;
  const [label, owner, conditions, sig] = tag as string[];
  if (label !== "auth" || !LHEX64.test(owner) || !LHEX128.test(sig) || !validConditions(conditions)) return null;
  const agent = agentPubkey.toLowerCase();
  if (owner === agent) return null; // self-attestation rejected
  try {
    const msg = sha256(new TextEncoder().encode(`nostr:agent-auth:${agent}:${conditions}`));
    return schnorr.verify(hexToBytes(sig), msg, hexToBytes(owner)) ? owner : null;
  } catch {
    return null;
  }
}

const cache = new Map<string, string | null>();
/** Verified NIP-OA owner of a kind:0 profile event (exactly one auth tag, valid event sig, conditions satisfied), else null. */
export function profileValidOaOwner(ev: NostrEvent): string | null {
  if (ev.kind !== 0) return null;
  const hit = cache.get(ev.id);
  if (hit !== undefined) return hit;
  const auth = ev.tags.filter((t) => t[0] === "auth");
  let out: string | null = null;
  if (auth.length === 1 && verifyEvent(ev as never)) {
    const owner = verifyAuthTag(auth[0], ev.pubkey);
    const cond = auth[0][2] ?? "";
    const applies =
      !cond ||
      cond.split("&").every((cl) =>
        cl.startsWith("kind=")
          ? Number(cl.slice(5)) === ev.kind
          : cl.startsWith("created_at<")
            ? ev.created_at < Number(cl.slice(11))
            : cl.startsWith("created_at>")
              ? ev.created_at > Number(cl.slice(11))
              : false,
      );
    out = owner && applies ? owner : null;
  }
  if (cache.size > 2000) cache.clear();
  cache.set(ev.id, out);
  return out;
}

/** Edits (kind 40003 + ["link-preview","none"]) by the author or the author's verified owner suppress previews. */
export function linkPreviewSuppressionTargets(
  originals: NostrEvent[],
  edits: NostrEvent[],
  owners: Map<string, string>,
): Set<string> {
  const byId = new Map(originals.map((e) => [e.id, e]));
  const out = new Set<string>();
  for (const edit of edits) {
    if (edit.kind !== 40003 || !edit.tags.some((t) => t.length === 2 && t[0] === "link-preview" && t[1] === "none")) continue;
    const target = edit.tags.find((t) => t[0] === "e")?.[1];
    const orig = target ? byId.get(target) : undefined;
    if (orig && (edit.pubkey === orig.pubkey || owners.get(orig.pubkey) === edit.pubkey)) out.add(orig.id);
  }
  return out;
}
export function ownersFromProfiles(profiles: NostrEvent[]): Map<string, string> {
  const m = new Map<string, string>();
  for (const p of profiles) {
    const o = profileValidOaOwner(p);
    if (o) m.set(p.pubkey, o);
  }
  return m;
}
export function applySuppression(tags: string[][], id: string, suppressed: Set<string>) {
  if (suppressed.has(id) && !tags.some((t) => t.length === 2 && t[0] === "link-preview" && t[1] === "none"))
    tags.push(["link-preview", "none"]);
}
