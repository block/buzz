// Web ports of the agent-observer crypto commands (src-tauri/src/commands/identity.rs:
// decrypt_observer_event, build_observer_control_event; crates/buzz-core observer.rs, buzz-sdk builders.rs).
// Frames are NIP-44 v2 between the agent key (event author) and the owner key (the web signer); the page never
// holds a secret key, so crypto goes through the signer's nip44 (cached by the instrumented signer).
import { verifyEvent } from "nostr-tools";
import { getSigner } from "./signer";
import { prioritizeDecrypt } from "./signer/instrument";

type Args = Record<string, any> | undefined;

const KIND_AGENT_OBSERVER_FRAME = 24200;
const NIP44_MIN_CONTENT_LEN = 132;
const NIP44_MAX_CONTENT_LEN = 87_472;
const OBSERVER_MAX_PLAINTEXT_LEN = 65_535;
const HEX64 = /^[0-9a-f]{64}$/;

const contentLooksLikeNip44 = (c: string) =>
  c.length >= NIP44_MIN_CONTENT_LEN && c.length <= NIP44_MAX_CONTENT_LEN;

export async function decryptObserverEvent(a: Args): Promise<unknown> {
  let ev: { id: string; pubkey: string; content: string; sig: string; kind: number; tags: string[][]; created_at: number };
  try {
    ev = JSON.parse(a?.eventJson ?? "");
  } catch (e) {
    throw new Error(`invalid event: ${String((e as Error)?.message ?? e)}`);
  }
  if (!ev || typeof ev.content !== "string" || !HEX64.test(String(ev.pubkey))) throw new Error("invalid event: malformed");
  // Defense-in-depth like desktop: verify id + signature before decrypting.
  if (!verifyEvent(ev as never)) throw new Error("observer event has invalid ID or signature");
  if (!contentLooksLikeNip44(ev.content))
    throw new Error(`decrypt observer event failed: invalid NIP-44 ciphertext length: ${ev.content.length}`);
  const n = getSigner().nip44;
  if (!n) throw new Error("Your signer does not support NIP-44 decryption.");
  // The Activity panel is on screen: don't queue behind settings decrypts (user-visible, priority 1).
  prioritizeDecrypt(ev.content);
  const plaintext = await n.decrypt(ev.pubkey, ev.content);
  if (plaintext.length > OBSERVER_MAX_PLAINTEXT_LEN)
    throw new Error(`decrypt observer event failed: observer plaintext exceeds ${OBSERVER_MAX_PLAINTEXT_LEN} bytes`);
  try {
    return JSON.parse(plaintext);
  } catch (e) {
    throw new Error(`decrypt observer event failed: JSON error: ${String((e as Error)?.message ?? e)}`);
  }
}

export async function buildObserverControlEvent(a: Args): Promise<string> {
  const agent = String(a?.agentPubkey ?? "").trim().toLowerCase();
  if (!HEX64.test(agent)) throw new Error("invalid agent pubkey");
  const s = getSigner();
  if (!s.nip44) throw new Error("Your signer does not support NIP-44 encryption.");
  const plaintext = JSON.stringify(a?.payload ?? null);
  if (plaintext.length > OBSERVER_MAX_PLAINTEXT_LEN) throw new Error("encrypt observer control failed: observer plaintext too large");
  const content = await s.nip44.encrypt(agent, plaintext);
  if (!contentLooksLikeNip44(content)) throw new Error("build observer control failed: observer frame content must be NIP-44 v2 ciphertext");
  const ev = await s.signEvent(
    {
      kind: KIND_AGENT_OBSERVER_FRAME,
      content,
      tags: [["p", agent], ["agent", agent], ["frame", "control"]],
    },
    { priority: 0 },
  );
  return JSON.stringify(ev);
}
