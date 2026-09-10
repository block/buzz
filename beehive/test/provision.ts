import { dirname } from 'node:path';
import { createGenesis } from '../src/assignment.ts';
import { publicKey } from '../src/protocol.ts';
import { provision, validateSetup } from '../src/host.ts';

/** Fresh fixture keys are explicitly enrolled once, just like local new-key setup. */
export function provisionSetup(path: string, value: unknown): void {
  const setup = validateSetup(value);
  provision(dirname(path), setup, createGenesis(publicKey(setup.ownerSecret), publicKey(setup.agentSecret), setup.host));
}
