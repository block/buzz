import { registerHooks } from 'node:module';
import { fileURLToPath } from 'node:url';
const fixture = fileURLToPath(new URL('./manager-installed-fixture.ts', import.meta.url));
// Explicit test-only seam. No production environment switch or OS-store fallback.
registerHooks({ load(url, context, next) {
  const result = next(url, context);
  if (!url.endsWith('/src/manager-entry.ts')) return result;
  const source = String(result.source);
  const original = 'new ManagerController(homedir(), snapshot => send({ snapshot }))';
  if (!source.includes(original)) throw Error('Installed manager fixture failed closed');
  return { ...result, source: `import { fixtureCredential, disconnectedTransport } from ${JSON.stringify(fixture)};\n` + source.replace(original, 'new ManagerController(homedir(), snapshot => send({ snapshot }), fixtureCredential, disconnectedTransport)') };
} });
