import { connectNostr } from './nostr-client.ts';
import { hostPairing, type HostPairing, type HostRegistration } from './host-registration.ts';
import { productionAdmission, type ScopedRelayAdmission } from './relay-admission.ts';
import { message, publicKey, type Message } from './protocol.ts';

/** Local host configuration pins the command signer, not an owner endorsement.
 * Legacy registration wrappers are compatibility input only (public history).
 * No agent grant is manufactured; retained per-agent journals remain authoritative.
 */
export function privateHostTransport(configuration: HostPairing | HostRegistration, secret: string, admission: ScopedRelayAdmission = productionAdmission) {
  const retained = { request: hostPairing('request' in configuration ? configuration.request : configuration) };
  if (publicKey(secret) !== retained.request.host) throw Error('Wrong infrastructure signer');
  return {
    availability: () => message('availability', retained.request.host, '', 0, { configuration: retained.request, observedAt: Date.now() }),
    binding: { host: retained.request.host, owner: retained.request.owner },
    validate(url: string) {
      if (url !== retained.request.relay) throw Error('Wrong host relay');
    },
    connect(url: string, _legacyOwnerSecret: string, receive: (m: Message) => void, _recovered?: () => void) {
      this.validate(url);
      admission.admit({ relay: url, publicKey: retained.request.host, transport: 'nip42-nip59', ownerDelegation: false });
      const wire = connectNostr(url, Buffer.from(secret, 'hex'), undefined, input => {
        const m = input.message;
        if (input.sender !== retained.request.owner || m.host !== retained.request.host || !['inspect', 'save', 'start', 'restart', 'stop'].includes(m.type)) return;
        receive(m);
      }, () => {}); // Existing durable host outbox survives; no automatic launch retry.
      return {
        ready: wire.ready,
        close: () => wire.close(),
        send(m: Message) {
          if (m.host !== retained.request.host || !['availability', 'inventory', 'receipt'].includes(m.type)) throw Error('Invalid private host report');
          void wire.publish(m, retained.request.owner).catch(() => {}); // Host outbox/heartbeat retains retry state.
        },
      };
    },
  };
}
