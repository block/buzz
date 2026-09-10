// Test-only module instrumentation: execute the real CLI, inject only the existing
// storage boundary. No production environment switch or copied shutdown owner.
import { registerHooks } from 'node:module';
registerHooks({ load(url, context, nextLoad) {
  const loaded = nextLoad(url, context);
  if (!url.endsWith('/src/relay.ts')) return loaded;
  const source = String(loaded.source);
  const needle = 'boundary?: SnapshotBoundary';
  if (!source.includes(needle)) throw Error('Relay boundary instrumentation drift');
  return { ...loaded, source: source.replace(needle, "boundary: SnapshotBoundary = async phase => { if (phase === 'directory') throw Error('injected directory fault'); }") };
} });
