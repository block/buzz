import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createGenesis } from '../src/assignment.ts';
import { hash, semanticHash, sameSelection, moveSelection, validateAssignment, type Grant } from '../src/handoff.ts';
import { message, newKey, publicKey } from '../src/protocol.ts';
import { profileRevision } from '../src/profiles.ts';

const reorder = (v: any): any => Array.isArray(v) ? v.map(reorder) : v !== null && typeof v === 'object' ? Object.fromEntries(Object.entries(v).reverse().map(([k,x]) => [k,reorder(x)])) : v;
test('validated grant selections are semantic; legacy identity hashes and ordered protocol arrays remain exact', () => {
  const genesis = createGenesis(publicKey(newKey()), publicKey(newKey()), 'source');
  const behavior = { name: 'Source', parent: null, instructions: 'Exact source instructions', revision: profileRevision('Source', null, 'Exact source instructions') };
  const requested = { model: 'fixture-model', workspace: '/fixture', profile: behavior.revision, configuration: { name: 'Destination', revision: 1 }, behavior };
  const operation = message('move', 'source', genesis.agent, 2, { target: 'target', targetRevision: 1, selection: reorder(requested) });
  const base: Grant = { root: hash(genesis), predecessor: hash(genesis), source: 'source', target: 'target', agent: genesis.agent, operation, prepared: 'immutable-token', selection: requested, targetRevision: 1, sourceRun: null };
  const validate = (g: Grant) => validateAssignment({ genesis, assignedHost: 'target', chain: [g] });
  validate(base); // Historical grants retain their existing hash and selection meaning.
  const grant: Grant = { ...base, materialization: 'named-v1', selection: moveSelection(requested, 1, 'named-v1') };
  validate(grant); validate({ ...grant, selection: reorder(grant.selection) });
  assert.ok(sameSelection(requested, reorder(requested)));
  assert.notEqual(hash(requested), hash(reorder(requested)));
  assert.equal(semanticHash(requested), semanticHash(reorder(requested)));
  assert.notEqual(semanticHash(['a','b']), semanticHash(['b','a']));
  for (const selection of [
    { ...grant.selection, workspace: '/other' }, { ...grant.selection, extra: true },
    { ...grant.selection, behavior: { ...behavior, extra: true } },
    { ...grant.selection, configuration: { name: 'Destination', revision: 1 } },
    { ...grant.selection, profile: 'default' },
    Object.fromEntries(Object.entries(grant.selection).filter(([k]) => k !== 'model')),
  ]) assert.throws(() => validate({ ...grant, selection } as Grant));
  assert.throws(() => validate({ ...grant, extra: true } as Grant));
  assert.throws(() => validate({ ...grant, materialization: 'unknown' } as any));
  const missing = { ...grant } as any; delete missing.prepared; assert.throws(() => validate(missing));
  const { configuration: _configuration, ...implicit } = requested;
  assert.deepEqual(moveSelection(implicit, 0, 'named-v1').configuration, { name: 'default', revision: 2 });
  assert.equal(moveSelection(requested, 0, 'named-v1').configuration!.revision, 2, 'initial projected @1 is never reused');
});
