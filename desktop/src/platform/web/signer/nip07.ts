// NIP-07 browser-extension signer (window.nostr).
import {
  type SignedEvent,
  type Signer,
  SignerUnavailableError,
  type UnsignedEvent,
} from "./types";

type Nip07 = {
  getPublicKey(): Promise<string>;
  signEvent(e: UnsignedEvent & { pubkey?: string }): Promise<SignedEvent>;
  nip44?: {
    encrypt(pk: string, pt: string): Promise<string>;
    decrypt(pk: string, ct: string): Promise<string>;
  };
};
const ext = () => (window as unknown as { nostr?: Nip07 }).nostr;

/** Extensions inject window.nostr asynchronously; poll briefly. */
export async function detectNip07(timeoutMs = 1500): Promise<boolean> {
  const end = Date.now() + timeoutMs;
  while (Date.now() < end) {
    if (ext()) return true;
    await new Promise((r) => setTimeout(r, 50));
  }
  return !!ext();
}

export function createNip07Signer(): Signer {
  const need = (): Nip07 => {
    const e = ext();
    if (!e)
      throw new SignerUnavailableError(
        "no-extension",
        "No NIP-07 browser extension (window.nostr) found.",
      );
    return e;
  };
  return {
    kind: "nip07",
    getPublicKey: () => need().getPublicKey(),
    async signEvent(ev) {
      const pubkey = await need().getPublicKey();
      const signed = await need().signEvent({
        ...ev,
        created_at: ev.created_at ?? Math.floor(Date.now() / 1000),
        pubkey,
      });
      return signed;
    },
    get nip44() {
      const n = ext()?.nip44;
      return (
        n && {
          encrypt: (p: string, t: string) => n.encrypt(p, t),
          decrypt: (p: string, c: string) => n.decrypt(p, c),
        }
      );
    },
  };
}
