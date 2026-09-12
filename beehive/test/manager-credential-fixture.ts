import { renameSync, writeFileSync } from 'node:fs';
process.once('message', (value: any) => {
  // Publish the PID atomically (staging file + rename) so the marker never
  // exists in a partially-published state: a reader that sees the marker must
  // see the complete PID, never an empty file (Number('') === 0 would probe the
  // caller's process group instead of the owned child). Same pattern as the
  // atomic blocker() fixture in credential-helper.test.ts.
  if (value.wait) { const pending = `${value.marker}.pending`; writeFileSync(pending, String(process.pid)); renameSync(pending, value.marker); setInterval(() => {}, 1000); }
  else { process.send?.({ ok: true, secret: null, node: process.versions.node, overrides: process.env.NODE_OPTIONS }); process.disconnect(); }
});
