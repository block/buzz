import { homedir } from 'node:os';
import { resolve } from 'node:path';

/** Resolve an explicitly supplied local path, including conventional ~/ spelling. */
export function setupPath(value: string): string {
  if (value.startsWith('~') && value !== '~' && !value.startsWith('~/')) throw Error('Use ~ or ~/ for your home; ~user paths are not supported');
  return resolve(value === '~' ? homedir() : value.startsWith('~/') ? `${homedir()}/${value.slice(2)}` : value);
}
