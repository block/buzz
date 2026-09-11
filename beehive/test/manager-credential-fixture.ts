import { writeFileSync } from 'node:fs';
process.once('message', (value: any) => {
  if (value.wait) { writeFileSync(value.marker, String(process.pid)); setInterval(() => {}, 1000); }
  else { process.send?.({ ok: true, secret: null, node: process.versions.node, overrides: process.env.NODE_OPTIONS }); process.disconnect(); }
});
