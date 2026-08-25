import assert from 'node:assert/strict';
import test from 'node:test';

import {
  EMBED_CODE_PATTERN,
  classifyEmbedSession,
  hashEmbedCode,
  safeEmbedTarget,
} from './embed-session-policy.mjs';

const now = 1_700_000_000_000;
const pending = {
  audience: 'business-dock',
  expires_at: now + 30_000,
  status: 'pending',
};

test('requires a 256-bit base64url one-time code', () => {
  assert.equal(EMBED_CODE_PATTERN.test('a'.repeat(43)), true);
  assert.equal(EMBED_CODE_PATTERN.test('a'.repeat(42)), false);
  assert.equal(EMBED_CODE_PATTERN.test(`${'a'.repeat(42)}+`), false);
  const plaintext = 'a'.repeat(43);
  const hash = hashEmbedCode(plaintext);
  assert.equal(hash.length, 43);
  assert.notEqual(hash, plaintext);
});

test('binds targets to the Business embed allowlist', () => {
  const origin = 'https://business.bizfin.test';
  assert.equal(safeEmbedTarget('/embed/sales/orders/SO-001', origin), '/embed/sales/orders/SO-001');
  for (const target of [
    'https://evil.example/embed/a',
    '//evil.example/embed/a',
    'javascript:alert(1)',
    'data:text/plain,no',
    'file:///tmp/no',
    '/embed/../admin',
    '/embed/a#secret',
  ]) assert.equal(safeEmbedTarget(target, origin), null);
});

test('accepts only a pending, unexpired, audience-bound record', () => {
  assert.equal(classifyEmbedSession(pending, now, 'business-dock'), 'valid');
  assert.equal(classifyEmbedSession(null, now, 'business-dock'), 'invalid');
  assert.equal(
    classifyEmbedSession(pending, now, 'different-audience'),
    'invalid',
  );
});

test('rejects replayed and expired records', () => {
  assert.equal(
    classifyEmbedSession({ ...pending, status: 'used' }, now, 'business-dock'),
    'used',
  );
  assert.equal(
    classifyEmbedSession({ ...pending, expires_at: now }, now, 'business-dock'),
    'expired',
  );
  assert.equal(
    classifyEmbedSession({ ...pending, status: 'revoked' }, now, 'business-dock'),
    'revoked',
  );
});
