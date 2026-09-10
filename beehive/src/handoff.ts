import { profile, type Profile } from './profiles.ts';
import { digest, fields, object, text, type Message, parseMessage } from './protocol.ts';
import { validateGenesis, type Genesis } from './assignment.ts';

/** Public exact launch selection; credentials and executable choices remain local. */
export type Selection = { model: string; workspace: string; profile: string; behavior?: Profile; configuration?: { name: string; revision: number } };
/** A source-consumed successor. Outer management authentication assumes trusted hosts. */
export type Grant = { root: string; predecessor: string; source: string; target: string; agent: string; operation: Message; prepared: string; selection: Selection; targetRevision: number; sourceRun: string | null };
export type Assignment = { genesis: Genesis; assignedHost: string; chain?: Grant[] };
export function hash(value: unknown): string { return digest(JSON.stringify(value)).toString('hex'); }
export function selection(value: unknown): Selection {
  const s = object(value); fields(s, ['model','workspace','profile', ...(s.behavior === undefined ? [] : ['behavior']), ...(s.configuration === undefined ? [] : ['configuration'])]);
  let configuration: Selection['configuration'];
  if (s.configuration !== undefined) {
    const c = object(s.configuration); fields(c, ['name', 'revision']);
    if (!Number.isSafeInteger(c.revision) || Number(c.revision) < 1) throw Error('Invalid configuration revision');
    configuration = { name: text(c.name, 64), revision: Number(c.revision) };
  }
  const behavior = s.behavior === undefined ? undefined : profile(s.behavior);
  if (s.profile !== 'default' && (!behavior || s.profile !== behavior.revision)) throw Error('Invalid profile reference');
  if (s.profile === 'default' && behavior) throw Error('Default cannot override instructions');
  return { model: text(s.model), workspace: text(s.workspace), profile: text(s.profile), ...(behavior ? { behavior } : {}), ...(configuration ? { configuration } : {}) };
}
/** Validate every predecessor, routing and immutable operation binding, not timestamps. */
export function validateAssignment(a: Assignment): void {
  const root = validateGenesis(a.genesis); let holder = root.initialHost, predecessor = hash(root);
  if (!Array.isArray(a.chain ?? []) || (a.chain?.length ?? 0) > 24) throw Error('Assignment chain limit');
  for (const g of a.chain ?? []) {
    fields(object(g), ['root','predecessor','source','target','agent','operation','prepared','selection','targetRevision','sourceRun']);
    const op = parseMessage(g.operation); fields(op.body, ['target','targetRevision','selection']);
    selection(g.selection); text(g.prepared);
    if (g.root !== hash(root) || g.predecessor !== predecessor || g.source !== holder || g.agent !== root.agent || g.target === holder || g.target !== op.body.target || op.host !== holder || op.agent !== root.agent || op.type !== 'move' || g.targetRevision !== op.body.targetRevision || hash(g.selection) !== hash(op.body.selection) || !Number.isSafeInteger(g.targetRevision) || g.targetRevision < 0 || (g.sourceRun !== null && typeof g.sourceRun !== 'string')) throw Error('Invalid consumed grant chain');
    text(g.target); holder = g.target; predecessor = hash(g);
  }
  if (a.assignedHost !== holder) throw Error('Saved assignment mismatch');
}
/** A recipient may only extend its durable knowledge, never accept historical authority. */
export function extendsAssignment(current: Assignment, next: Assignment): boolean {
  validateAssignment(next);
  return hash(current.genesis) === hash(next.genesis) && (next.chain?.length ?? 0) > (current.chain?.length ?? 0) && (current.chain ?? []).every((g, i) => hash(g) === hash(next.chain![i]));
}
