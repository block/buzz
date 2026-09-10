import { digest, fields, object, text, type Message } from './protocol.ts';

/** Nonsecret immutable behavior; no launch settings or management authority. */
export type Profile = { name: string; parent: string | null; instructions: string; revision: string };
const revisionPattern = /^[a-f0-9]{64}$/;
/** Canonical content address, independent of object property order. */
export function profileRevision(name: string, parent: string | null, instructions: string) {
  return digest(JSON.stringify(['beehive-profile-v1', name, parent, instructions])).toString('hex');
}
/** Strict public profile codec. Newlines are instructions, never executable arguments. */
export function profile(value: unknown): Profile {
  const p = object(value); fields(p, ['name', 'parent', 'instructions', 'revision']);
  const name = text(p.name, 80);
  if (name === 'default' || (p.parent !== null && (typeof p.parent !== 'string' || !revisionPattern.test(p.parent)))) throw Error('Invalid profile name/parent');
  if (typeof p.instructions !== 'string' || !p.instructions.trim() || Buffer.byteLength(p.instructions) > 2048 || /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/.test(p.instructions)) throw Error('Invalid nonsecret instructions');
  const parent = p.parent as string | null;
  const revision = profileRevision(name, parent, p.instructions);
  if (p.revision !== revision) throw Error('Profile revision conflict');
  return { name, parent, instructions: p.instructions, revision };
}
/** Read-only projection of authenticated relay publications. No mutable latest head.
 * Missing parents remain incomplete; descendants cannot become executable by arrival order.
 */
export class Profiles {
  private versions = new Map<string, Profile>();
  receive(m: Message) {
    if (m.type !== 'profile') return;
    try {
      if (m.host !== 'profiles' || m.agent !== 'profiles' || m.revision !== 0) throw Error('Invalid profile routing');
      const p = profile(m.body);
      if (!this.versions.has(p.revision) && this.versions.size >= 1000) throw Error('Profile catalog full');
      this.versions.set(p.revision, Object.freeze(p));
    } catch { /* Invalid publications never enter the executable catalog. */ }
  }
  resolve(revision: string): Profile {
    const p = this.versions.get(revision);
    if (!p) throw Error('Profile revision unavailable');
    let ancestor = p; const seen = new Set<string>();
    while (ancestor.parent !== null) {
      if (seen.size >= 100 || seen.has(ancestor.revision)) throw Error('Profile lineage limit');
      seen.add(ancestor.revision);
      const parent = this.versions.get(ancestor.parent);
      if (!parent || parent.name !== p.name) throw Error('Profile lineage incomplete/conflicting');
      ancestor = parent;
    }
    return structuredClone(p);
  }
  /** Incomplete versions stay visible but cannot be selected for execution. */
  incomplete(): Profile[] {
    return [...this.versions.values()].filter(p => { try { this.resolve(p.revision); return false; } catch { return true; } }).map(p => structuredClone(p));
  }
  list(): Profile[] {
    return [...this.versions.keys()].flatMap(id => { try { return [this.resolve(id)]; } catch { return []; } });
  }
}
