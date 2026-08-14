import { nip19 } from "nostr-tools";

import { KIND_LONG_FORM } from "@/shared/constants/kinds";

export const NIP19_MAX_LENGTH = 5000;

export type LongFormAddress = {
  kind: typeof KIND_LONG_FORM;
  pubkey: string;
  identifier: string;
  relays: string[];
};

export type LongFormNaddr = LongFormAddress & { url: string };

export function parseNaddrUri(uri: string): LongFormAddress | null {
  const parsed = parseLongFormNaddr(uri);
  if (!parsed) {
    return null;
  }

  return {
    kind: parsed.kind,
    pubkey: parsed.pubkey,
    identifier: parsed.identifier,
    relays: parsed.relays,
  };
}

function parseLongFormNaddr(value: string): LongFormNaddr | null {
  const url = value.trim();
  if (!url.startsWith("nostr:naddr1") || url.length > NIP19_MAX_LENGTH) {
    return null;
  }

  try {
    const decoded = nip19.decode(url.slice("nostr:".length));
    if (decoded.type !== "naddr" || decoded.data.kind !== KIND_LONG_FORM) {
      return null;
    }
    if (
      !decoded.data.identifier.trim() ||
      !/^[0-9a-f]{64}$/i.test(decoded.data.pubkey)
    ) {
      return null;
    }

    return {
      identifier: decoded.data.identifier,
      kind: KIND_LONG_FORM,
      pubkey: decoded.data.pubkey.toLowerCase(),
      relays: decoded.data.relays ?? [],
      url,
    };
  } catch {
    return null;
  }
}

export function isLongFormNaddr(value: string): boolean {
  return parseNaddrUri(value) !== null;
}
