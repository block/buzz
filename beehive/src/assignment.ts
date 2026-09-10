import { randomUUID } from 'node:crypto';
import { fields, object, text } from './protocol.ts';

/** Public, locally pinned root. It conveys no secret and is not a bearer grant. */
export type Genesis = { v: 1; id: string; owner: string; agent: string; initialHost: string };
/** Only new-key creation or verified one-time legacy enrollment may mint a root. */
export function createGenesis(owner: string, agent: string, initialHost: string): Genesis {
  return validateGenesis({ v: 1, id: randomUUID(), owner, agent, initialHost });
}
/** Validate exact public root fields before binding any local execution authority. */
export function validateGenesis(value: unknown): Genesis {
  const g = object(value);
  fields(g, ['v', 'id', 'owner', 'agent', 'initialHost']);
  if (g.v !== 1 || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(text(g.id))) throw Error('Invalid assignment genesis');
  for (const key of ['owner', 'agent']) if (!/^[0-9a-f]{64}$/.test(text(g[key]))) throw Error('Invalid genesis identity');
  text(g.initialHost);
  return g as Genesis;
}
