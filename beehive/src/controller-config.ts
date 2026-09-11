import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { fields, object } from './protocol.ts';
import { readPrivate, writePrivate } from './storage.ts';
import { ownerPublicInput } from './host-setup.ts';

export type ControllerConfig = { version: 1; owner: string; relay: string };
const file = (directory: string) => join(directory, 'controller.json');

/** Read public controller routing only. The owner signer remains in OS credentials. */
export function readControllerConfig(directory: string): ControllerConfig | undefined {
  if (!existsSync(file(directory))) return undefined;
  const value = object(readPrivate(file(directory)));
  fields(value, ['version', 'owner', 'relay']);
  if (value.version !== 1) throw Error('Unsupported Beehive controller configuration');
  const owner = ownerPublicInput(String(value.owner));
  const relay = String(value.relay);
  if (!/^(wss|ws):\/\//.test(relay)) throw Error('Invalid retained management relay URL');
  return { version: 1, owner, relay };
}

/** First-use public routing configuration. Existing owner/relay authority is immutable here. */
export function createControllerConfig(directory: string, owner: string, relay: string): ControllerConfig {
  if (readControllerConfig(directory)) throw Error('Controller already configured; use Settings for deliberate changes');
  const value = { version: 1 as const, owner: ownerPublicInput(owner), relay: relay.trim() };
  if (!/^(wss|ws):\/\//.test(value.relay)) throw Error('Management relay requires wss:// (ws://127.0.0.1 for fixtures)');
  writePrivate(file(directory), value, true);
  return value;
}
