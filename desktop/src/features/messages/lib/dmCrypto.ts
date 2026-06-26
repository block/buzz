import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_EDIT,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";
import { nip44DecryptFromPeer } from "@/shared/api/tauri";
import type { Channel, RelayEvent } from "@/shared/api/types";

/**
 * Body shown when a DM message is valid NIP-44 v2 ciphertext we cannot decrypt
 * (peer used a key/version we can't read) — fail-visible, never blank or
 * garbled. Substituted ONLY for undecryptable valid-v2 content; malformed or
 * legacy-plaintext content is passed through untouched (see
 * `decryptIngestedContent`).
 */
export const UNDECRYPTABLE_DM_PLACEHOLDER =
  "[encrypted message — update your client to read it]";

/**
 * Kinds that carry a user-authored free-text body in a DM and must therefore be
 * encrypted on send and decrypted at ingest. System messages, reactions, and
 * deletions carry no peer-encrypted body and are excluded.
 *
 * The relay's `is_e2e_enforced_content_kind` covers a broader set (pinned,
 * bookmarked, scheduled, reminder, diff, forum, canvas) but only kinds the
 * desktop client currently sends/receives in DMs are listed here. This list
 * must grow in lockstep if new content kinds are added to DMs.
 */
function isDmContentKind(kind: number): boolean {
  return (
    kind === KIND_STREAM_MESSAGE ||
    kind === KIND_STREAM_MESSAGE_V2 ||
    kind === KIND_STREAM_MESSAGE_EDIT
  );
}

/**
 * The single NIP-44 peer for a 2-party DM: the one participant that isn't us.
 * Returns null when the channel isn't a 2-party DM (open/stream channels,
 * self-only, or group DMs with >1 other participant), which is exactly the
 * scope where FE peer crypto applies — callers skip encrypt/decrypt on null.
 */
export function dmPeerPubkey(
  channel: Pick<Channel, "channelType" | "participantPubkeys">,
  selfPubkey: string | undefined,
): string | null {
  if (channel.channelType !== "dm" || !selfPubkey) {
    return null;
  }
  const self = normalizePubkey(selfPubkey);
  const peers = channel.participantPubkeys.filter(
    (pubkey) => normalizePubkey(pubkey) !== self,
  );
  return peers.length === 1 ? peers[0] : null;
}

/**
 * Whether `content` is a syntactically plausible NIP-44 v2 ciphertext payload.
 *
 * Mirrors the relay's `buzz_core::observer::validate_nip44_v2` envelope check
 * exactly so the FE's "this IS encrypted, we just can't read it" judgement
 * matches the boundary the relay enforces:
 * - standard base64 alphabet, padding only at the end, length a multiple of 4
 * - decoded length >= 99 bytes (1 version + 32 nonce + 32 MAC + >=34 ciphertext)
 * - first decoded byte is 0x02 (NIP-44 version 2)
 *
 * It is an envelope check, not decryption: a `true` result means the content is
 * shaped like v2 ciphertext, so a decrypt failure is "valid ciphertext we can't
 * read" (→ placeholder) rather than "this was never encrypted" (→ pass through).
 */
export function looksLikeNip44V2(content: string): boolean {
  const len = content.length;
  if (len === 0 || len % 4 !== 0) {
    return false;
  }

  let padCount = 0;
  for (let i = 0; i < len; i++) {
    const c = content[i];
    if (c === "=") {
      // Padding is only legal in the final two positions.
      if (i < len - 2) {
        return false;
      }
      padCount++;
      if (padCount > 2) {
        return false;
      }
    } else if (/[A-Za-z0-9+/]/.test(c)) {
      // A base64 char after padding has begun is malformed.
      if (padCount > 0) {
        return false;
      }
    } else {
      return false;
    }
  }

  const decodedLen = (len / 4) * 3 - padCount;
  if (decodedLen < 99) {
    return false;
  }

  const b64Val = (c: string): number => {
    const code = c.charCodeAt(0);
    if (code >= 65 && code <= 90) return code - 65; // A-Z
    if (code >= 97 && code <= 122) return code - 97 + 26; // a-z
    if (code >= 48 && code <= 57) return code - 48 + 52; // 0-9
    if (c === "+") return 62;
    if (c === "/") return 63;
    return -1;
  };

  const firstByte = (b64Val(content[0]) << 2) | (b64Val(content[1]) >> 4);
  return firstByte === 0x02;
}

/**
 * Decrypt one ingested DM event's content to plaintext for the cache.
 *
 * Decrypt is attempted only for 2-party-DM content kinds whose content is
 * shaped like NIP-44 v2 ciphertext (`looksLikeNip44V2`). Everything else —
 * non-DM channels, system/reaction/deletion kinds, and legacy plaintext that
 * predates encryption — is returned unchanged.
 *
 * When the content IS valid v2 ciphertext but `decrypt` throws (peer key/version
 * we can't read), the fail-visible placeholder is substituted. This is the AC1
 * distinction: a decrypt failure on valid-v2 content shows the placeholder,
 * while content that was never v2-shaped is passed through as-is.
 */
export async function decryptIngestedContent(
  event: Pick<RelayEvent, "kind" | "content">,
  peerPubkey: string | null,
  decrypt: (peerPubkey: string, ciphertext: string) => Promise<string>,
): Promise<string> {
  if (
    peerPubkey === null ||
    !isDmContentKind(event.kind) ||
    !looksLikeNip44V2(event.content)
  ) {
    return event.content;
  }

  try {
    return await decrypt(peerPubkey, event.content);
  } catch {
    return UNDECRYPTABLE_DM_PLACEHOLDER;
  }
}

/**
 * Build the decrypt-at-ingest mapper for a channel: it decrypts encrypted
 * 2-party-DM bodies to plaintext (via the real Tauri NIP-44 peer primitive) and
 * leaves everything else — non-DM channels, non-content kinds, legacy plaintext
 * — untouched. Returns a no-op identity mapper outside a 2-party DM, so every
 * ingest site can call it unconditionally without branching on channel type.
 *
 * Callers pass this to the cache-population paths (history fetch, scrollback,
 * aux backfill, live append) so the cache only ever holds plaintext content.
 */
export function makeDmIngestDecryptor(
  channel: Pick<Channel, "channelType" | "participantPubkeys"> | null,
  selfPubkey: string | undefined,
): (events: RelayEvent[]) => Promise<RelayEvent[]> {
  const peerPubkey = channel ? dmPeerPubkey(channel, selfPubkey) : null;
  if (peerPubkey === null) {
    return (events) => Promise.resolve(events);
  }
  return (events) =>
    Promise.all(
      events.map(async (event) => {
        const content = await decryptIngestedContent(
          event,
          peerPubkey,
          nip44DecryptFromPeer,
        );
        return content === event.content ? event : { ...event, content };
      }),
    );
}
