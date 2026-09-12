import { provisionCredentialSlot } from './credential-slots.ts';
import { validateSetup } from './host.ts';
import { validateGenesis } from './assignment.ts';
import { readPrivate } from './storage.ts';
import { object } from './protocol.ts';
import { readHostIdentity, bootstrapHostIdentity } from './host-identity.ts';
import { createCredential, credentialReference, readCredential, systemCredentials } from './credential-store.ts';
import { publicKey } from './protocol.ts';
import { registerAgent, agentNsec, addOpenAI, providerCredentials } from './settings-credentials.ts';
import { readSettings } from './settings.ts';
import { openAIModels } from './settings-models.ts';

// Node-only, bounded by the controller. Secrets use private IPC, never argv/output.
process.once('message', async (input: any) => {
  try {
    if (input.action === 'configure') {
      bootstrapHostIdentity(input.directory, input.label, input.owner, input.relay);
      process.send?.({ ok: true });
    } else if (input.action === 'provision') {
      const identity = readHostIdentity(input.directory);
      const binding = object(readPrivate(input.binding));
      if (['host', 'ownerSecret', 'ownerPublic', 'agentSecret'].some(key => Object.hasOwn(binding, key))) throw Error('Identity in binding');
      const setup = validateSetup({ ...binding, host: identity.pairing.host, ownerPublic: identity.pairing.owner });
      provisionCredentialSlot(input.directory, setup, input.secret, validateGenesis(readPrivate(input.genesis)));
      process.send?.({ ok: true });
    } else if (input.action === 'register-agent') {
      const secret = agentNsec(input.secret);
      registerAgent(input.directory, secret, systemCredentials, input.profile);
      process.send?.({ ok: true });
    } else if (input.action === 'add-openai') {
      addOpenAI(input.directory,input.name,input.secret,providerCredentials());
      process.send?.({ ok: true });
    } else if (input.action === 'provider-read') {
      const secret = providerCredentials().read(input.key);
      if (!secret) throw Error('Missing provider credential');
      process.send?.({ ok: true, secret });
    } else if (input.action === 'models') {
      const provider = readSettings(input.directory).providers.find(p => p.id === input.provider);
      if (!provider) throw Error('Missing provider');
      const secret = providerCredentials().read(provider.key);
      if (!secret) throw Error('Missing provider credential');
      const models = await openAIModels(secret, AbortSignal.timeout(7000));
      process.send?.({ ok: true, models });
    } else if (input.action === 'signin') {
      const ref = credentialReference('owner', input.owner);
      if (input.secret !== undefined) {
        if (publicKey(input.secret) !== input.owner) throw Error('mismatch');
        createCredential('owner', input.secret);
      }
      const secret = systemCredentials.read(ref) === null ? null : readCredential(ref);
      process.send?.({ ok: true, secret });
    } else throw Error('Unsupported operation');
  } catch { process.send?.({ ok: false }); }
  finally { process.disconnect(); }
});
