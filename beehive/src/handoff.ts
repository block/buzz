import { profile, type Profile } from './profiles.ts';
import { digest, fields, object, text, type Message, parseMessage } from './protocol.ts';
import { validateGenesis, type Genesis } from './assignment.ts';

/** Public exact launch selection; credentials and executable choices remain local. */
export type Selection = { harnessSetup?: { id: string; fingerprint: string }; model: string; workspace: string; profile: string; behavior?: Profile; configuration?: { name: string; revision: number } };
/** A source-consumed successor. Outer management authentication assumes trusted hosts. */
export type Grant = { root: string; predecessor: string; source: string; target: string; agent: string; operation: Message; prepared: string; selection: Selection; targetRevision: number; sourceRun: string | null; materialization?: 'named-v1' };
export type Assignment = { genesis: Genesis; assignedHost: string; chain?: Grant[] };
export function hash(value: unknown): string { return digest(JSON.stringify(value)).toString('hex'); }
export function selection(value: unknown): Selection {
  const s = object(value); fields(s, ['model','workspace','profile', ...(s.harnessSetup === undefined ? [] : ['harnessSetup']), ...(s.behavior === undefined ? [] : ['behavior']), ...(s.configuration === undefined ? [] : ['configuration'])]);
  let configuration: Selection['configuration'];
  if (s.configuration !== undefined) {
    const c = object(s.configuration); fields(c, ['name', 'revision']);
    if (!Number.isSafeInteger(c.revision) || Number(c.revision) < 1) throw Error('Invalid configuration revision');
    configuration = { name: text(c.name, 64), revision: Number(c.revision) };
  }
  let harnessSetup: Selection['harnessSetup'];
  if (s.harnessSetup !== undefined) {
    const ref = object(s.harnessSetup); fields(ref, ['id', 'fingerprint']);
    if (!/^[0-9a-f]{64}$/.test(String(ref.fingerprint))) throw Error('Invalid binding fingerprint');
    harnessSetup = { id: text(ref.id, 64), fingerprint: String(ref.fingerprint) };
  }
  const behavior = s.behavior === undefined ? undefined : profile(s.behavior);
  if (s.profile !== 'default' && (!behavior || s.profile !== behavior.revision)) throw Error('Invalid profile reference');
  if (s.profile === 'default' && behavior) throw Error('Default cannot override instructions');
  return { ...(harnessSetup ? { harnessSetup } : {}), model: text(s.model), workspace: text(s.workspace), profile: text(s.profile), ...(behavior ? { behavior } : {}), ...(configuration ? { configuration } : {}) };
}
/** JSON value canonicalization for local prepared-input equality, never signed event
 * identity. Arrays retain order; every object member remains represented. */
export function semanticHash(value: unknown): string {
  const canonical = (v: any): any => Array.isArray(v) ? v.map(canonical)
    : v !== null && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map(k => [k, canonical(v[k])])) : v;
  return hash(canonical(value));
}
/** Semantic comparison only for strictly validated public selections. Raw hash remains
 * the identity of historical grants, operations and immutable retry envelopes. */
export function sameSelection(a: unknown, b: unknown): boolean {
  return hash(selection(a)) === hash(selection(b));
}
/** Compare only target-local launch inputs, after validating all behavior fields. */
export function sameLaunchSelection(a: unknown, b: unknown): boolean {
  const local = (value: unknown) => { const { behavior: _behavior, ...s } = selection(value); return { ...s, profile: 'default' }; };
  return sameSelection(local(a), local(b));
}
/** Versioned Move contract: the requested destination reference is a CAS input,
 * not the identity of the resulting source-behavior/target-launch snapshot. */
export function moveSelection(value: unknown, targetRevision: number, materialization?: 'named-v1'): Selection {
  const next = selection(value);
  if (materialization !== undefined && materialization !== 'named-v1') throw Error('Invalid Move materialization');
  if (materialization) {
    // Legacy initial selected-next has no explicit reference; its catalog projects
    // default@1. New Moves materialize that name too, never split the projection.
    next.configuration ??= { name: 'default', revision: 1 };
    if (!Number.isSafeInteger(targetRevision + 1) || !Number.isSafeInteger(next.configuration.revision + 1) || targetRevision < 0) throw Error('Invalid Move revision');
    next.configuration = { name: next.configuration.name, revision: Math.max(targetRevision + 1, next.configuration.revision + 1) };
  }
  return next;
}
/** Validate every predecessor, routing and immutable operation binding, not timestamps. */
export function validateAssignment(a: Assignment): void {
  const root = validateGenesis(a.genesis); let holder = root.initialHost, predecessor = hash(root);
  if (!Array.isArray(a.chain ?? []) || (a.chain?.length ?? 0) > 24) throw Error('Assignment chain limit');
  for (const g of a.chain ?? []) {
    fields(object(g), ['root','predecessor','source','target','agent','operation','prepared','selection','targetRevision','sourceRun', ...(g.materialization === undefined ? [] : ['materialization'])]);
    const op = parseMessage(g.operation); fields(op.body, ['target','targetRevision','selection']);
    selection(g.selection); text(g.prepared);
    if (g.root !== hash(root) || g.predecessor !== predecessor || g.source !== holder || g.agent !== root.agent || g.target === holder || g.target !== op.body.target || op.host !== holder || op.agent !== root.agent || op.type !== 'move' || g.targetRevision !== op.body.targetRevision || !sameSelection(g.selection, moveSelection(op.body.selection, g.targetRevision, g.materialization)) || !Number.isSafeInteger(g.targetRevision) || g.targetRevision < 0 || (g.sourceRun !== null && typeof g.sourceRun !== 'string')) throw Error('Invalid consumed grant chain');
    text(g.target); holder = g.target; predecessor = hash(g);
  }
  if (a.assignedHost !== holder) throw Error('Saved assignment mismatch');
}
/** A recipient may only extend its durable knowledge, never accept historical authority. */
export function extendsAssignment(current: Assignment, next: Assignment): boolean {
  validateAssignment(next);
  return hash(current.genesis) === hash(next.genesis) && (next.chain?.length ?? 0) > (current.chain?.length ?? 0) && (current.chain ?? []).every((g, i) => hash(g) === hash(next.chain![i]));
}
