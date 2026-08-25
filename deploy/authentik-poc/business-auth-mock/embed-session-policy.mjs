import { createHash } from 'node:crypto';

export const EMBED_CODE_PATTERN = /^[A-Za-z0-9_-]{43}$/;

export function hashEmbedCode(value) {
  return createHash('sha256').update(value).digest('base64url');
}

export function safeEmbedTarget(value, businessOrigin) {
  try {
    const target = new URL(value || '/', businessOrigin);
    if (
      target.origin !== businessOrigin ||
      target.username ||
      target.password ||
      target.hash ||
      (target.pathname !== '/' && !target.pathname.startsWith('/embed/'))
    ) return null;
    return `${target.pathname}${target.search}`;
  } catch {
    return null;
  }
}

export function classifyEmbedSession(row, now, expectedAudience) {
  if (!row || row.audience !== expectedAudience) return 'invalid';
  if (row.status === 'revoked') return 'revoked';
  if (row.status !== 'pending') return 'used';
  if (!Number.isFinite(Number(row.expires_at)) || Number(row.expires_at) <= now)
    return 'expired';
  return 'valid';
}
