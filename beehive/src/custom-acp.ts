import { isAbsolute } from 'node:path';
import { lstatSync } from 'node:fs';
import { readPrivate } from './storage.ts';
/** Owner-local immutable definition. Contract is explicit opt-in, never inferred from ACP version. */
export type CustomAcp = { id: string; label: string; executable: string; args: string[]; env: Record<string, string>; installHint: string; installInstructionsUrl: string; contract: 'diagnostic' | 'goose-native' };
export const diagnosticReason = 'Setup/diagnostic only: no supported exact actual-session model/native profile contract; Start/Restart unavailable.';
/** Validate structured argv/env without comma splitting or shell interpretation. */
export function validateCustom(value: unknown): CustomAcp {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw Error('Invalid custom ACP definition');
  const d = value as CustomAcp;
  if (Object.keys(d).some(k => !['id','label','executable','args','env','installHint','installInstructionsUrl','contract'].includes(k))) throw Error('Unknown custom field; install scripts/capability metadata are not supported');
  const string = (v: unknown, max: number) => typeof v === 'string' && v.length <= max && !/[\x00-\x1f\x7f]/.test(v);
  if (!string(d.id, 64) || !/^[a-z0-9_][a-z0-9_-]*$/.test(d.id) || ['constructor','prototype','__proto__'].includes(d.id) || !string(d.label, 120) || !d.label || !string(d.executable, 4096) || !isAbsolute(d.executable)) throw Error('Custom ACP requires local id/label and absolute executable');
  if (!Array.isArray(d.args) || d.args.length > 128 || d.args.some(a => !string(a, 4096))) throw Error('Custom argv must be a bounded JSON string array');
  if (!d.env || typeof d.env !== 'object' || Array.isArray(d.env) || Object.keys(d.env).length > 64) throw Error('Custom env must be a bounded JSON object');
  for (const [k, v] of Object.entries(d.env)) {
    if (!/^[A-Z_][A-Z0-9_]{0,99}$/.test(k) || /^(BUZZ_|GOOSE_|CODEX_|ANTHROPIC_|CLAUDE_|NODE_|LD_|DYLD_)/.test(k) || ['HOME','PATH','SHELL','ENV','BASH_ENV','NODE_OPTIONS','OPENAI_API_KEY'].includes(k) || !string(v, 8192)) throw Error('Custom env contains reserved/invalid key or value');
  }
  if (!string(d.installHint, 4096) || !string(d.installInstructionsUrl, 2048) || !['diagnostic','goose-native'].includes(d.contract)) throw Error('Unsupported custom contract or setup hints');
  if (d.installInstructionsUrl) {
    let u: URL; try { u = new URL(d.installInstructionsUrl); } catch { throw Error('Invalid setup documentation URL'); }
    if (u.protocol !== 'https:' || u.username || u.password) throw Error('Setup documentation must be credential-free HTTPS');
  }
  if (Buffer.byteLength(JSON.stringify(d)) > 65536) throw Error('Custom definition too large');
  return structuredClone(d);
}
/** Only fresh owner-controlled regular 0600 input; no credential contents displayed. */
export function readCustom(path: string): CustomAcp {
  if (!isAbsolute(path)) throw Error('Custom definition file must be absolute');
  const s = lstatSync(path);
  if (s.size > 65536 || (process.getuid && s.uid !== process.getuid())) throw Error('Custom input must be bounded and owned by the service user');
  let value: unknown;
  try { value = readPrivate(path); } catch (e) { if (e instanceof SyntaxError) throw Error('Malformed custom JSON (contents withheld)'); throw e; }
  return validateCustom(value);
}
