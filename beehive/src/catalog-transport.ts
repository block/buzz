import { connectNostr } from './nostr-client.ts';
import { HostOffers, catalogHost, catalogResponse, verifyHostCatalog, type HostCatalog } from './host-catalog.ts';
import { open, publicKey, type Envelope, type Message } from './protocol.ts';

/** Adapter for the existing durable owner client. No broadcast, OA, or label routing.
 */
export function catalogTransport(catalog: HostCatalog) {
  return (url: string, secret: string, receive: (m: Message) => void, recovered?: () => void, disconnected?: (code: number) => void) => {
    const retained = structuredClone(verifyHostCatalog(catalog, publicKey(secret), url));
    const offers = new HostOffers();
    let closed = false;
    let generation = 0;
    function dial() {
      const current = ++generation;
      return connectNostr(url, Buffer.from(secret, 'hex'), input => {
        if (closed || current !== generation) return;
        let m: Message;
        try { m = catalogResponse(retained, input, Math.floor(Date.now() / 1000), offers); } catch { return; }
        receive(m);
      }, () => { if (!closed && current === generation) disconnected?.(1008); });
    }
    let wire = dial();
    function send(m: Message) {
      if (closed) throw Error('UI closed');
      // Profiles need explicit per-host distribution and receipts before enabling.
      if (!['inspect', 'save', 'start', 'restart', 'stop'].includes(m.type)) throw Error('Operation not integrated with private host transport');
      const host = catalogHost(retained, m.host, Math.floor(Date.now() / 1000), offers);
      const current = generation;
      void wire.publish(m, host.request.host).catch(() => { if (!closed && current === generation) disconnected?.(1008); });
    }
    return {
      ready: wire.ready,
      get socket() { return wire.socket; },
      send,
      // At-rest envelope is local-only. Fresh wraps preserve its exact inner ID/body.
      sendEnvelope(envelope: Envelope) { send(open(envelope, secret)); },
      reconnect() {
        if (closed) throw Error('UI closed');
        const previous = wire;
        wire = dial(); previous.close();
        void wire.ready.then(() => recovered?.()).catch(() => {}); // disconnect callback retains UNKNOWN.
      },
      close() { closed = true; ++generation; wire.close(); },
    };
  };
}
