import { randomUUID } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { fields, object } from './protocol.ts';
import { profile, profileRevision, type Profile } from './profiles.ts';
import { readPrivate, writePrivate } from './storage.ts';

export type ProfileDraft = { id: string; value: Profile };
/** Local editable text only; no signer, relay, host, model or key access. Each
 * completed input is an atomic save, NOT keystroke durability. A short directory
 * lock refuses concurrent editors rather than silently overwriting their saves.
 */
export function profileDrafts(stateDirectory: string) {
  const directory = join(stateDirectory, 'profile-drafts');
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  for (const path of [stateDirectory, directory]) {
    const stat = lstatSync(path);
    if (!stat.isDirectory() || (stat.mode & 0o077)) throw Error('Draft directory must be owner-only');
  }
  const path = join(directory, 'drafts.json');
  function list(): ProfileDraft[] {
    if (!existsSync(path)) return [];
    if (lstatSync(path).size > 1000000) throw Error('Draft storage exceeds limit');
    const stored = object(readPrivate(path)); fields(stored, ['version', 'drafts']);
    if (stored.version !== 1 || !Array.isArray(stored.drafts) || stored.drafts.length > 100) throw Error('Invalid draft storage');
    const ids = new Set<string>();
    return stored.drafts.map(input => {
      const d = object(input); fields(d, ['id', 'value']);
      if (typeof d.id !== 'string' || !/^[0-9a-f-]{36}$/.test(d.id) || ids.has(d.id)) throw Error('Invalid draft identity');
      ids.add(d.id); return { id: d.id, value: profile(d.value) };
    });
  }
  function change(update: (drafts: ProfileDraft[]) => ProfileDraft[]) {
    const lock = join(directory, 'edit.lock');
    mkdirSync(lock, { mode: 0o700 });
    try { writePrivate(path, { version: 1, drafts: update(list()) }); }
    finally { rmdirSync(lock); }
  }
  return {
    list,
    save(value: Profile, previous?: ProfileDraft): ProfileDraft {
      const draft = { id: previous?.id ?? randomUUID(), value: profile(value) };
      change(drafts => {
        if (previous) {
          const index = drafts.findIndex(d => d.id === previous.id);
          if (index < 0 || drafts[index]!.value.revision !== previous.value.revision) throw Error('Draft changed; resume again');
          drafts[index] = draft;
        } else { if (drafts.length >= 100) throw Error('Draft storage full; discard unused drafts'); drafts.push(draft); }
        return drafts;
      });
      return draft;
    },
    discard(previous: ProfileDraft) {
      change(drafts => {
        if (!drafts.some(d => d.id === previous.id && d.value.revision === previous.value.revision)) throw Error('Draft changed; resume again');
        return drafts.filter(d => d.id !== previous.id);
      });
    },
  };
}

/** Shared keyless editor for the offline command and the connected owner TUI. */
export async function editProfileDraft(store: ReturnType<typeof profileDrafts>, question: (prompt: string) => Promise<string>, previous?: ProfileDraft, parent?: Profile): Promise<ProfileDraft> {
  const name = previous?.value.name ?? parent?.name ?? await question('Profile name: ');
  const instructions = await question('Nonsecret behavior instructions (no provider/model/credentials): ');
  const ancestor = previous?.value.parent ?? parent?.revision ?? null;
  const value = profile({ name, parent: ancestor, instructions, revision: profileRevision(name, ancestor, instructions) });
  return store.save(value, previous);
}
