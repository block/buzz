import { serveHost } from './host-service.ts';
import { host } from './host.ts';
import { readHostIdentityAsync } from './host-identity.ts';
import { systemCredentials } from './credential-store.ts';
import { credentialHelperReader } from './credential-helper.ts';
import { privateHostTransport } from './host-transport.ts';

// Detached Node only. No key material in argv, stdout, logs or service records.
const directory = process.argv[2]!, expected = process.argv[3]!;
try {
  await serveHost(directory,expected,async () => {
    const credentials = { ...systemCredentials, readAsync: credentialHelperReader({ operatorApproved: true }) };
    const identity = await readHostIdentityAsync(directory,credentials);
    const transport = privateHostTransport(identity.pairing,identity.secret);
    const running = await host(directory,identity.pairing.relay,undefined,transport,credentials);
    return { close: () => running.close(), status: () => ({ revision: running.settingsRevision, agents: running.agents.length, relay: transport.connected ? 'connected' as const : 'disconnected' as const }) };
  });
} catch { process.exitCode = 1; }
