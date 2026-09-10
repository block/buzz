import { connectNostr } from './nostr-client.ts';
import { verifyHostRegistration, type HostRegistration } from './host-registration.ts';
import { productionAdmission, type ScopedRelayAdmission } from './relay-admission.ts';
import { publicKey, type Message } from './protocol.ts';

/** Private infrastructure transport for the existing host executor. No agent grant
 * is manufactured here; its retained per-agent journal remains the authority owner.
 */
export function privateHostTransport(registration: HostRegistration, secret: string, admission: ScopedRelayAdmission = productionAdmission) {
  const retained = structuredClone(verifyHostRegistration(registration, registration.request));
  if (publicKey(secret) !== retained.request.host) throw Error('Wrong infrastructure signer');
  return {
    binding: { host: retained.request.host, owner: retained.request.owner },
    validate(url: string) {
      verifyHostRegistration(retained, retained.request);
      if (url !== retained.request.relay) throw Error('Wrong host relay');
      admission.admit({ relay: url, publicKey: retained.request.host, transport: 'nip42-nip59', ownerDelegation: false });
    },
    connect(url: string, _legacyOwnerSecret: string, receive: (m: Message) => void, _recovered?: () => void) {
      this.validate(url);
      const wire = connectNostr(url, Buffer.from(secret, 'hex'), undefined, input => {
        try { verifyHostRegistration(retained, retained.request); } catch { return; }
        const m = input.message;
        if (input.sender !== retained.request.owner || m.host !== retained.request.host || !['inspect', 'save', 'start', 'restart', 'stop'].includes(m.type)) return;
        receive(m);
      }, () => {}); // Existing durable host outbox survives; no automatic launch retry.
      return {
        ready: wire.ready,
        close: () => wire.close(),
        send(m: Message) {
          verifyHostRegistration(retained, retained.request);
          if (m.host !== retained.request.host || !['inventory', 'receipt'].includes(m.type)) throw Error('Invalid private host report');
          void wire.publish(m, retained.request.owner).catch(() => {}); // Host outbox/heartbeat retains retry state.
        },
      };
    },
  };
}
