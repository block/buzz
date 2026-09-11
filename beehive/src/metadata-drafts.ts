import { randomUUID } from 'node:crypto';
import { existsSync, lstatSync, mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { fields, object } from './protocol.ts';
import { publicMetadata, metadataRevision, metadataRelay, type PublicMetadata } from './public-metadata.ts';
import { readPrivate, writePrivate } from './storage.ts';

export type MetadataDraft = { id: string; relay: string; agent: string; value: PublicMetadata };
/** Local editable text only; no signer, relay, host, model or key access. Each
 * completed input is an atomic save, NOT keystroke durability. A short directory
 * lock refuses concurrent editors rather than silently overwriting their saves.
 */
export function metadataDrafts(stateDirectory: string) {
  const directory = join(stateDirectory, 'metadata-drafts');
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  for (const path of [stateDirectory, directory]) {
    const stat = lstatSync(path);
    if (!stat.isDirectory() || (stat.mode & 0o077)) throw Error('Draft directory must be owner-only');
  }
  const path = join(directory, 'drafts.json');
  function list(): MetadataDraft[] {
    if (!existsSync(path)) return [];
    if (lstatSync(path).size > 1000000) throw Error('Draft storage exceeds limit');
    const stored = object(readPrivate(path)); fields(stored, ['version', 'drafts']);
    if (stored.version !== 1 || !Array.isArray(stored.drafts) || stored.drafts.length > 100) throw Error('Invalid draft storage');
    const ids = new Set<string>();
    return stored.drafts.map(input => {
      const d = object(input); fields(d, ['id', 'relay', 'agent', 'value']);
      if (typeof d.id !== 'string' || !/^[0-9a-f-]{36}$/.test(d.id) || ids.has(d.id)) throw Error('Invalid draft identity');
      if (typeof d.relay !== 'string' || typeof d.agent !== 'string' || !/^[a-f0-9]{64}$/.test(d.agent)) throw Error('Invalid draft binding');
      metadataRelay(d.relay);
      ids.add(d.id); return { id: d.id, relay: d.relay, agent: d.agent, value: publicMetadata(d.value) };
    });
  }
  function change(update: (drafts: MetadataDraft[]) => MetadataDraft[]) {
    const lock = join(directory, 'edit.lock');
    mkdirSync(lock, { mode: 0o700 });
    try { writePrivate(path, { version: 1, drafts: update(list()) }); }
    finally { rmdirSync(lock); }
  }
  return {
    list,
    save(relay: string, agent: string, value: PublicMetadata, previous?: MetadataDraft): MetadataDraft {
      metadataRelay(relay); if (!/^[a-f0-9]{64}$/.test(agent)) throw Error('Agent must be a hex public key');
      if (previous && (previous.relay !== relay || previous.agent !== agent)) throw Error('Draft binding cannot change');
      const draft = { id: previous?.id ?? randomUUID(), relay, agent, value: publicMetadata(value) };
      change(drafts => {
        if (previous) {
          const index = drafts.findIndex(d => d.id === previous.id);
          if (index < 0 || metadataRevision(drafts[index]!.value) !== metadataRevision(previous.value)) throw Error('Draft changed; resume again');
          drafts[index] = draft;
        } else { if (drafts.length >= 100) throw Error('Draft storage full; discard unused drafts'); drafts.push(draft); }
        return drafts;
      });
      return draft;
    },
    discard(previous: MetadataDraft) {
      change(drafts => {
        if (!drafts.some(d => d.id === previous.id && metadataRevision(d.value) === metadataRevision(previous.value))) throw Error('Draft changed; resume again');
        return drafts.filter(d => d.id !== previous.id);
      });
    },
  };
}

/** Owner-only completed-form save; blank values deliberately clear fields. */
export async function editMetadataDraft(store: ReturnType<typeof metadataDrafts>, question: (prompt: string) => Promise<string>, previous?: MetadataDraft): Promise<MetadataDraft> {
  if (previous) console.log(`Saved public draft ${previous.id}: ${JSON.stringify(previous.value)}`);
  const relay = previous?.relay ?? await question('Public relay (exact retained ws/wss URL): ');
  const agent = previous?.agent ?? await question('Agent hex public key: ');
  const display_name = await question('Public display name (blank clears): ');
  const picture = await question('Public HTTPS picture URL (blank clears): ');
  const about = await question('Public about (NOT private instructions; blank clears): ');
  return store.save(relay, agent, { display_name, picture, about }, previous);
}
