// Playwright helper: injects a mock NIP-07 extension (window.nostr). Secret key stays in node, like a real extension.
import { finalizeEvent, getPublicKey, nip44 } from "nostr-tools";
const hexToBytes = (h) => new Uint8Array(Buffer.from(h, "hex"));

export async function installMockExtension(
  context,
  skHex,
  { withNip44 = true, delay = 0 } = {},
) {
  const sk = hexToBytes(skHex);
  const pk = getPublicKey(sk);
  const log = { signed: [] };
  await context.exposeFunction("__nostrSign", (ev) => {
    const s = finalizeEvent(
      {
        kind: ev.kind,
        content: ev.content,
        tags: ev.tags,
        created_at: ev.created_at,
      },
      sk,
    );
    log.signed.push(s.kind);
    return s;
  });
  await context.exposeFunction("__nostrEnc", (peer, pt) =>
    nip44.v2.encrypt(pt, nip44.v2.utils.getConversationKey(sk, peer)),
  );
  await context.exposeFunction("__nostrDec", (peer, ct) =>
    nip44.v2.decrypt(ct, nip44.v2.utils.getConversationKey(sk, peer)),
  );
  await context.addInitScript(
    ({ pk, withNip44, delay }) => {
      const wait = () => new Promise((r) => setTimeout(r, delay));
      window.nostr = {
        getPublicKey: async () => {
          await wait();
          return pk;
        },
        signEvent: async (e) => {
          await wait();
          return window.__nostrSign(e);
        },
        ...(withNip44
          ? {
              nip44: {
                encrypt: (p, t) => window.__nostrEnc(p, t),
                decrypt: (p, c) => window.__nostrDec(p, c),
              },
            }
          : {}),
      };
    },
    { pk, withNip44, delay },
  );
  return { pk, log };
}
