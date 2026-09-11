import { profile } from './profiles.ts';
import { connectNostr } from './nostr-client.ts';
import { HostOffers, catalogHost, catalogResponse, verifyHostCatalog, type HostCatalog } from './host-catalog.ts';
import { fields, open, publicKey, type Envelope, type Message } from './protocol.ts';

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
        try {
          if (input.message.type === 'profile') {
            m = input.message;
            if (input.sender !== retained.owner || m.host !== 'profiles' || m.agent !== 'profiles' || m.revision !== 0) return;
            fields(m.body, ['relay', 'profile']);
            if (m.body.relay !== url) return;
            m = { ...m, body: profile(m.body.profile) };
          } else m = catalogResponse(retained, input, Math.floor(Date.now() / 1000), offers);
        } catch { return; }
        receive(m);
      }, () => { if (!closed && current === generation) disconnected?.(1008); });
    }
    let wire = dial();
    function send(m: Message) {
      if (closed) throw Error('UI closed');
      // The owner's library is addressed only to the owner. Hosts receive the
      // selected snapshot in an independently authenticated, CAS-fenced Save.
      if (m.type === 'profile') {
        profile(m.body);
        if (m.host !== 'profiles' || m.agent !== 'profiles' || m.revision !== 0) throw Error('Invalid profile routing');
        const current = generation;
        void wire.publish({ ...m, body: { relay: url, profile: m.body } }, retained.owner).catch(() => { if (!closed && current === generation) disconnected?.(1008); });
        return;
      }
      if (!['inspect', 'save', 'start', 'restart', 'stop'].includes(m.type)) throw Error('Operation not integrated with private host transport');
      const host = catalogHost(retained, m.host, Math.floor(Date.now() / 1000), offers);
      const current = generation;
      void wire.publish(m, host.request.host).catch(() => { if (!closed && current === generation) disconnected?.(1008); });
    }
    return {
      ready: wire.ready,
      get socket() { return wire.socket; },
      send,
      // At-rest envelope is local-only. Profile wire bodies additionally bind the relay;
      // decoding removes only that verified scope wrapper. Operation identity is unchanged.
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
